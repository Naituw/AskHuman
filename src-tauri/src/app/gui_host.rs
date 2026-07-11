//! 统一 GUI 宿主进程的运行时（spec D2/D5/D10/D11）：单实例托盘图标 + 设置/历史/Agent 窗口。
//!
//! 由 `app::run_gui_host` 经 `app::launch(View::GuiHost)` 进入；本模块负责：
//! - 单实例 flock（`gui-host.lock`）+ macOS 活动策略（有图标时 accessory）。
//! - 自有 IPC 监听（`gui-host.sock`）：收 `OpenWindow` → 主线程聚焦/新建唯一窗口。
//! - daemon 客户端：一条**非保活**的 `TraySubscribe`（驱动图标/菜单），外加「有窗口时」一条
//!   普通连接给 daemon 续命（spec D5）。
//! - 配置监听：菜单栏模式 / 语言变化 → 重建菜单 + 装/卸登录项 + 切活动策略。
//! - 二进制换新：盘上二进制变化且无窗口时 re-exec / 交 launchd（spec D11）。

#![cfg(unix)]

use crate::app::tray_menu::{Node, TrayMenu};
use crate::config::{AppConfig, DaemonLifecycleMode, MenuBarIconMode};
use crate::daemon::lifecycle::{self, Fingerprint, LockGuard};
use crate::gui_host::{HostMsg, WindowKind};
use crate::i18n::{self, Lang};
use crate::ipc::{self, transport, ClientMsg, ServerMsg};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::Menu;
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager};
use tokio::io::BufReader;
use tokio::sync::Notify;

const QUICK_ASK_INTERJECT: &str =
    "I have a new thought. Before continuing, ask me immediately using the questioning tool described in your instructions.";

/// 某 label 是否为宿主统一承载的窗口（用于窗口计数 / 续命判定）。
/// 插话 composer 窗口每 session 一个，label 动态（`interject-<hash>`），按前缀识别。
pub fn is_hosted_label(label: &str) -> bool {
    matches!(label, "settings" | "history" | "agents") || label.starts_with("interject-")
}

// ===== 内嵌图标资源（三态；统一单色模板图）=====
//
// 三态图标由 `scripts/gen_tray_icons.py` 从设计稿 `icons/tray/source.png`
// 抠图生成：黑色像素=不透明，白色（如 "?"、月牙缺口）=透明。macOS 作为
// 模板图（系统按菜单栏明暗自动反色）；Linux 直接使用同一张单色图。
mod icon_bytes {
    pub const IDLE: &[u8] = include_bytes!("../../icons/tray/tray-idle.png");
    pub const ACTIVE: &[u8] = include_bytes!("../../icons/tray/tray-active.png");
    pub const STOPPED: &[u8] = include_bytes!("../../icons/tray/tray-stopped.png");
    /// 仅 macOS 把单色图当作模板图染色；其它平台原样显示。
    pub const TEMPLATE: bool = cfg!(target_os = "macos");
}

// ===== 单实例锁（进程级；re-exec 时显式释放）=====

static HOST_LOCK: OnceLock<Mutex<Option<LockGuard>>> = OnceLock::new();

fn host_lock_slot() -> &'static Mutex<Option<LockGuard>> {
    HOST_LOCK.get_or_init(|| Mutex::new(None))
}

/// 获取宿主单实例锁；成功返回 true（并持有锁至进程退出 / 显式释放）。
pub fn acquire_singleton() -> bool {
    match lifecycle::acquire_lock_at(&crate::paths::gui_host_lock()) {
        Ok(Some(g)) => {
            *host_lock_slot().lock().unwrap() = Some(g);
            true
        }
        _ => false,
    }
}

/// 释放单实例锁（仅二进制换新 re-exec 前调用，让新实例可抢锁）。
fn release_lock() {
    if let Ok(mut s) = host_lock_slot().lock() {
        *s = None;
    }
}

// ===== 宿主运行态（Tauri managed）=====

#[derive(Clone, Default)]
pub struct TrayData {
    pub running: bool,
    pub version: String,
    pub uptime_secs: u64,
    pub active_requests: usize,
    pub im_connections: Vec<String>,
    pub draining: bool,
    pub agents_working: usize,
    pub agents_idle: usize,
    pub update_available: bool,
    pub update_latest: String,
    pub pending: bool,
    /// 在途请求摘要（托盘「待答」子菜单逐条列出）。
    pub pending_requests: Vec<ipc::PendingRequestInfo>,
    /// 活动 agent 摘要（托盘「Agent 状态」子菜单逐条列出，spec agent-interject D7）。
    pub agents: Vec<ipc::TrayAgentInfo>,
}

pub struct HostState {
    pub mode: Mutex<MenuBarIconMode>,
    /// 上次已应用的守护进程生命周期模式：用于在配置变更时检测「切到保活」的跃迁，
    /// 从而立即拉起 daemon（打开开关即视为一次触发，见 spec）。
    pub daemon_lifecycle: Mutex<DaemonLifecycleMode>,
    pub lang: Mutex<Lang>,
    pub data: Mutex<TrayData>,
    pub daemon_up: AtomicBool,
    pub windows_open: AtomicUsize,
    /// 是否曾经打开过窗口（off/active 模式退出判定用，避免开窗前误退）。
    pub ever_open: AtomicBool,
    /// 启动宽限是否已过（覆盖「OpenWindow 始终未到达」的兜底退出）。
    pub grace_over: AtomicBool,
    pub tray: Mutex<Option<TrayIcon>>,
    /// 持久托盘菜单（**长期持有同一个** `Menu` 对象 + 影子树）：刷新时按 `key` 做 diff，只就地改条目
    /// 文字 / 可用性，结构变化才最小增删——绝不整段重建或 `set_menu` 换对象（spec 菜单稳定性）。
    pub tray_menu: Mutex<Option<TrayMenu>>,
    /// 上次渲染的「菜单/图标内容签名」：与本次相同则整次刷新直接跳过（连 diff 都不做）——
    /// daemon 持续停止时内容不变 → 不触碰菜单 → 展开的菜单不会被挤掉。
    pub menu_sig: Mutex<Option<String>>,
    /// 窗口期续命连接的停止信号（持有即有续命连接在）。
    pub keepalive: Mutex<Option<Arc<Notify>>>,
    /// Agent 状态订阅的停止信号（与 agent 窗口绑定）：开窗（前端就绪）时重启、关窗时停。
    /// 长命宿主下若复用旧订阅，daemon 不会重推首帧 → 窗口首屏长时间 Loading，故每次开窗都重启。
    pub agents_sub: Mutex<Option<Arc<Notify>>>,
    /// 启动时的二进制指纹（盘上内容变化即触发宿主换新）。
    pub startup_fp: Fingerprint,
}

impl HostState {
    fn mode(&self) -> MenuBarIconMode {
        *self.mode.lock().unwrap()
    }
    fn lang(&self) -> Lang {
        *self.lang.lock().unwrap()
    }
}

/// 当前平台是否支持托盘（macOS 恒真；Linux 需图形会话）。
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

// ===== 入口：在 launch() 的 setup 中调用 =====

/// 在 Tauri setup（主线程）中初始化宿主：活动策略 + 托盘 + IPC + daemon 客户端 + 配置监听。
pub fn setup(app: &mut tauri::App, config: &AppConfig) -> tauri::Result<()> {
    let lang = Lang::resolve(&config.general.language);
    let mode = config.general.menu_bar_icon;

    app.manage(HostState {
        mode: Mutex::new(mode),
        daemon_lifecycle: Mutex::new(config.general.daemon_lifecycle),
        lang: Mutex::new(lang),
        data: Mutex::new(TrayData::default()),
        daemon_up: AtomicBool::new(false),
        windows_open: AtomicUsize::new(0),
        ever_open: AtomicBool::new(false),
        grace_over: AtomicBool::new(false),
        tray: Mutex::new(None),
        tray_menu: Mutex::new(None),
        menu_sig: Mutex::new(None),
        keepalive: Mutex::new(None),
        agents_sub: Mutex::new(None),
        startup_fp: lifecycle::current_fingerprint(),
    });

    // 初始活动策略（macOS）：有图标 → accessory（不占 Dock/Cmd-Tab）；off → regular（窗口正常入坞）。
    #[cfg(target_os = "macos")]
    {
        let policy = if mode == MenuBarIconMode::Off {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        app.set_activation_policy(policy);

        // tao launched() 在 setup hook 之前以 .regular + activateIgnoringOtherApps(true)
        // 激活了应用。对无窗口的状态栏 app，这导致 macOS 在首次展开子菜单时隐式关闭菜单。
        // 详见 docs/investigations/tray-menu-close-on-first-hover.md
        if mode != MenuBarIconMode::Off {
            let mtm = objc2_foundation::MainThreadMarker::new().unwrap();
            let ns_app = objc2_app_kit::NSApp(mtm);
            ns_app.deactivate();
            #[allow(deprecated)]
            ns_app.activateIgnoringOtherApps(false);
        }
    }

    // 有图标且托盘可用 → 建托盘（初始 idle 图标，随后由状态订阅刷新）。
    if mode != MenuBarIconMode::Off && tray_supported() {
        let _ = ensure_tray(&app.handle().clone(), true);
    }
    // always 模式装登录项（best-effort）。
    if mode == MenuBarIconMode::Always {
        let _ = crate::integrations::login_item::ensure_installed();
    }

    let handle = app.handle().clone();
    start_ipc_listener(handle.clone());
    start_status_subscription(handle.clone());
    start_config_watch(handle.clone());
    start_binary_watch(handle.clone());

    // 启动宽限：12s 后标记并复核退出（覆盖 OpenWindow 始终未到达的极端情况）。
    let h = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(12)).await;
        if let Some(state) = h.try_state::<HostState>() {
            state.grace_over.store(true, Ordering::SeqCst);
        }
        evaluate_exit(&h);
    });

    Ok(())
}

/// macOS 活动策略：有图标（菜单栏 app）→ accessory（不占 Dock/Cmd-Tab）；off → regular（窗口正常入坞）。
fn apply_activation_policy(app: &AppHandle, mode: MenuBarIconMode) {
    #[cfg(target_os = "macos")]
    {
        let policy = if mode == MenuBarIconMode::Off {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, mode);
    }
}

// ===== 托盘图标 / 菜单 =====

fn decode_icon(bytes: &'static [u8]) -> Option<Image<'static>> {
    Image::from_bytes(bytes).ok()
}

fn icon_for(daemon_up: bool, active_requests: usize) -> Option<Image<'static>> {
    let bytes = if !daemon_up {
        icon_bytes::STOPPED
    } else if active_requests > 0 {
        icon_bytes::ACTIVE
    } else {
        icon_bytes::IDLE
    };
    decode_icon(bytes)
}

/// 建立（present=true）或移除（present=false）托盘图标。须在主线程调用。
fn ensure_tray(app: &AppHandle, present: bool) -> tauri::Result<()> {
    let Some(state) = app.try_state::<HostState>() else {
        return Ok(());
    };
    if !present {
        *state.tray.lock().unwrap() = None; // Drop → 移除托盘
        *state.tray_menu.lock().unwrap() = None; // 菜单随托盘失效；下次重建时再新建并填充
        *state.menu_sig.lock().unwrap() = None;
        return Ok(());
    }
    if state.tray.lock().unwrap().is_some() {
        refresh_tray(app);
        return Ok(());
    }
    // 建一个空菜单挂上托盘并长期持有；条目交给 refresh_tray 经 diff 填充（之后只就地改 / 最小增删）。
    let menu = Menu::new(app)?;
    let mut builder = TrayIconBuilder::with_id("askhuman-tray")
        .icon_as_template(icon_bytes::TEMPLATE)
        .menu(&menu)
        .show_menu_on_left_click(true);
    if let Some(img) = decode_icon(icon_bytes::IDLE) {
        builder = builder.icon(img);
    }
    let tray = builder.build(app)?;
    *state.tray.lock().unwrap() = Some(tray);
    *state.tray_menu.lock().unwrap() = Some(TrayMenu::new(app.clone(), menu));
    // 强制首刷：清掉签名缓存，确保 refresh_tray 一定填充菜单 + 摆正图标/tooltip。
    *state.menu_sig.lock().unwrap() = None;
    refresh_tray(app);
    Ok(())
}

/// 按最近状态刷新图标 + 菜单 + tooltip。须在主线程调用。
///
/// 关键：**内容没变就整次跳过**（连菜单条目都不碰）；有变化时也只**原地增删菜单条目**，
/// 绝不 `tray.set_menu` 换新对象。这样 daemon 持续停止（内容不变）时展开的菜单不会被 2s
/// 刷新挤掉；即便有变化，原地改条目通常也不会关闭已展开菜单（只有替换 NSMenu 对象才会）。
pub fn refresh_tray(app: &AppHandle) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    let tray_guard = state.tray.lock().unwrap();
    let Some(tray) = tray_guard.as_ref() else {
        return;
    };
    let up = state.daemon_up.load(Ordering::SeqCst);
    let data = state.data.lock().unwrap().clone();
    let lang = state.lang();
    // 是否有任一家开启了生命周期追踪：未开启时隐藏 Agent 状态相关菜单项（入口 + 忙闲行）。
    let lifecycle_on = crate::integrations::agent_lifecycle::any_installed();

    // 内容签名：与上次相同 → 整次跳过，不触碰托盘（连 diff 都省，确保展开的菜单纹丝不动）。
    let sig = menu_signature(up, lang, &data, lifecycle_on);
    if state.menu_sig.lock().unwrap().as_deref() == Some(sig.as_str()) {
        return;
    }

    // 图标（set_icon / set_tooltip 不会关闭已展开菜单）。
    if let Some(img) = icon_for(up, data.active_requests) {
        let _ = tray.set_icon(Some(img));
        let _ = tray.set_icon_as_template(icon_bytes::TEMPLATE);
    }
    // 菜单：把期望节点列表 diff 应用到**同一个**菜单对象——文字变化只 set_text、结构变化才最小增删，
    // 绝不整段重建（整段重建会关掉已展开菜单）。
    if let Some(tm) = state.tray_menu.lock().unwrap().as_mut() {
        tm.apply(build_specs(up, lang, &data, lifecycle_on));
    }
    let tip = if up {
        i18n::tr(lang, "tray.tooltipRunning").to_string()
    } else {
        i18n::tr(lang, "tray.tooltipStopped").to_string()
    };
    let _ = tray.set_tooltip(Some(&tip));

    *state.menu_sig.lock().unwrap() = Some(sig);
}

/// 决定菜单/图标渲染结果的全部输入拼成的签名：与上次相同即「整次跳过」（菜单已是正确状态，连 diff 都省，
/// 确保展开的菜单纹丝不动）；不同才进入 diff。**必须覆盖 `build_specs` 与图标/tooltip 的每个输入**，
/// 否则真变化会被误跳过。uptime 取分钟级文案，避免秒级微变把每次推送都判为「有变化」。
fn menu_signature(up: bool, lang: Lang, data: &TrayData, lifecycle_on: bool) -> String {
    // 待答子菜单内容（id+预览）也入签名：列表/预览变化即触发 diff。
    let pending: String = data
        .pending_requests
        .iter()
        .map(|p| format!("{}={}", p.id, p.preview))
        .collect::<Vec<_>>()
        .join(";");
    // Agent 子菜单内容也入签名：会话增删 / 标题 / 状态 / 待送达 / 可聚焦变化即触发 diff。
    let agents: String = data
        .agents
        .iter()
        .map(|a| {
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{}",
                a.session_id,
                a.seq,
                a.kind,
                a.title,
                a.project_name,
                a.state,
                a.pending_interject as u8,
                a.focusable as u8
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    format!(
        "{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        lang,
        up as u8,
        data.version,
        fmt_uptime(data.uptime_secs),
        data.draining as u8,
        data.active_requests,
        data.agents_working,
        data.agents_idle,
        data.im_connections.join(","),
        data.update_available as u8,
        // update_latest 仅在 update_available 时入签名，避免无更新时的噪声变化触发刷新。
        if data.update_available {
            data.update_latest.as_str()
        } else {
            ""
        },
        lifecycle_on as u8,
        data.pending as u8,
        pending,
        agents,
    )
}

fn fmt_uptime(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

/// 生成「期望的托盘菜单节点列表」（声明式，spec D7）：状态区只读条目 + 操作区可点条目。
/// 每个节点带稳定 `key`（可点条目的 `key` 即事件路由 id）；由 `TrayMenu::apply` diff 应用——
/// 文字变化只 `set_text`、结构变化才最小增删，绝不整段重建（整段重建会关掉已展开菜单）。
fn build_specs(up: bool, lang: Lang, data: &TrayData, lifecycle_on: bool) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();

    // —— 状态区（只读）——
    let title = if up {
        i18n::tr(lang, "tray.running").to_string()
    } else {
        i18n::tr(lang, "tray.stopped").to_string()
    };
    nodes.push(Node::item("st.title", title, false));
    if up {
        nodes.push(Node::item(
            "st.version",
            i18n::tr(lang, "tray.version").replace("{v}", &data.version),
            false,
        ));
        nodes.push(Node::item(
            "st.uptime",
            i18n::tr(lang, "tray.uptime").replace("{d}", &fmt_uptime(data.uptime_secs)),
            false,
        ));
        if data.draining {
            nodes.push(Node::item(
                "st.draining",
                i18n::tr(lang, "tray.draining").to_string(),
                false,
            ));
        }
        // 忙闲数量不再单列只读行——已合并进操作区「Agent 状态」入口的标题（见下方 open_agents）。
        if !data.im_connections.is_empty() {
            nodes.push(Node::item(
                "st.im",
                i18n::tr(lang, "tray.imConnections")
                    .replace("{list}", &data.im_connections.join(", ")),
                false,
            ));
        }
    }
    if data.update_available {
        nodes.push(Node::item(
            "st.update_avail",
            i18n::tr(lang, "tray.updateAvailable").replace("{v}", &data.update_latest),
            false,
        ));
    }
    if data.pending {
        nodes.push(Node::item(
            "st.update_pending",
            i18n::tr(lang, "tray.updatePending").to_string(),
            false,
        ));
    }

    // —— 操作区 ——
    nodes.push(Node::separator("sep.actions"));
    // 「待答」放在操作区最前、独立一段——它是唯一可点的状态项，混在上方一堆灰色只读行里显得乱。
    if up && data.active_requests > 0 {
        let title = i18n::tr(lang, "tray.pendingQuestions")
            .replace("{n}", &data.active_requests.to_string());
        // 有逐条摘要 → 子菜单（点击聚焦对应弹窗）；缺摘要（旧 daemon）→ 退回只读计数行。
        if data.pending_requests.is_empty() {
            nodes.push(Node::item("st.pending_count", title, false));
        } else {
            let children = data
                .pending_requests
                .iter()
                .map(|p| {
                    let label = if p.preview.is_empty() {
                        i18n::tr(lang, "tray.pendingUntitled").to_string()
                    } else {
                        p.preview.clone()
                    };
                    Node::item(format!("focus_req:{}", p.id), label, true)
                })
                .collect();
            nodes.push(Node::submenu("st.pending_menu", title, true, children));
        }
        nodes.push(Node::separator("sep.pending"));
    }
    nodes.push(Node::item(
        "open_settings",
        i18n::tr(lang, "tray.openSettings").to_string(),
        true,
    ));
    nodes.push(Node::item(
        "open_history",
        i18n::tr(lang, "tray.openHistory").to_string(),
        true,
    ));
    // 「Agent 状态」入口仅在开启了生命周期追踪时显示——否则窗口必为空，徒增困惑。
    // 忙闲数量直接并入标题（合并了原状态区的只读忙闲行）。
    // 有活动 agent（daemon 下发摘要）时父项变**子菜单**（spec agent-interject D7）：
    // 首项「打开状态窗口」+ 分隔线 + 逐 agent 子菜单（发送消息 / 聚焦终端；工作中在前）；
    // 无活动 agent / 旧 daemon（缺摘要）→ 退回普通条目（点击即开窗口）。
    if lifecycle_on {
        let label = if up && data.agents_working + data.agents_idle > 0 {
            i18n::tr(lang, "tray.openAgentsCounts")
                .replace("{w}", &data.agents_working.to_string())
                .replace("{i}", &data.agents_idle.to_string())
        } else {
            i18n::tr(lang, "tray.openAgents").to_string()
        };
        if !up || data.agents.is_empty() {
            nodes.push(Node::item("open_agents", label, true));
        } else {
            let mut children = vec![Node::item(
                "open_agents",
                i18n::tr(lang, "tray.openAgentsWindow").to_string(),
                true,
            )];
            children.push(Node::separator("sep.agents"));
            for a in &data.agents {
                // Agent 条目前缀用与 /watch 卡片一致的状态圆点，避免仅靠排序区分工作中/空闲。
                // 编号可直接用于 `/msg <编号>`；标题截断 24 字符防菜单过宽。
                let project = if a.project_name.is_empty() {
                    i18n::tr(lang, "autoChannel.noProject").to_string()
                } else {
                    a.project_name.clone()
                };
                let session_title = if a.title.trim().is_empty() {
                    i18n::tr(lang, "autoChannel.noTitle").to_string()
                } else {
                    truncate_chars(a.title.trim(), AGENT_TITLE_MAX_CHARS)
                };
                let title = format!(
                    "{} · [{}] {} — {}（{}）",
                    agent_state_label(&a.state, lang),
                    a.seq,
                    agent_kind_label(&a.kind),
                    session_title,
                    project
                );
                let mut sub: Vec<Node> = Vec::new();
                // 「发送消息」：grok 无可靠传话通道（首期排除，spec agent-interject D1），且仅「工作中」
                // 才显示——插话在 agent 下一次工具调用时送达，对空闲无意义（用户定案）。
                if a.kind != "grok" && a.state == "working" {
                    sub.push(Node::item(
                        format!("ijask:{}", a.session_id),
                        i18n::tr(lang, "tray.agentAskNow").to_string(),
                        true,
                    ));
                    let text = if a.pending_interject {
                        i18n::tr(lang, "tray.agentSendMessagePending").to_string()
                    } else {
                        i18n::tr(lang, "tray.agentSendMessage").to_string()
                    };
                    sub.push(Node::item(format!("ij:{}", a.session_id), text, true));
                }
                if a.focusable {
                    sub.push(Node::item(
                        format!("term:{}", a.session_id),
                        i18n::tr(lang, "tray.agentFocusTerminal").to_string(),
                        true,
                    ));
                }
                if sub.is_empty() {
                    // 无任何可用动作（grok 且终端不可聚焦）：列为只读行，仅供感知。
                    children.push(Node::item(format!("agent:{}", a.session_id), title, false));
                } else {
                    children.push(Node::submenu(
                        format!("agent:{}", a.session_id),
                        title,
                        true,
                        sub,
                    ));
                }
            }
            nodes.push(Node::submenu("agents_menu", label, true, children));
        }
    }
    nodes.push(Node::separator("sep.update"));
    nodes.push(Node::item(
        "check_update",
        i18n::tr(lang, "tray.checkUpdate").to_string(),
        true,
    ));
    if data.update_available {
        nodes.push(Node::item(
            "apply_update",
            i18n::tr(lang, "tray.applyUpdate").replace("{v}", &data.update_latest),
            true,
        ));
    }
    nodes.push(Node::separator("sep.daemon"));
    if up {
        nodes.push(Node::item(
            "restart_daemon",
            i18n::tr(lang, "tray.restartDaemon").to_string(),
            true,
        ));
        // 有「工作中」agent 时「停止」无意义：daemon 一停，agent 的生命周期 hook（report_agent_event
        // → ensure_running）或下次 ask 会几秒内把它重新拉起。故隐藏停止项，仅留一行灰色说明。
        if data.agents_working > 0 {
            nodes.push(Node::item(
                "st.stop_blocked",
                i18n::tr(lang, "tray.stopDaemonBlocked").to_string(),
                false,
            ));
        } else {
            nodes.push(Node::item(
                "stop_daemon",
                i18n::tr(lang, "tray.stopDaemon").to_string(),
                true,
            ));
        }
    } else {
        nodes.push(Node::item(
            "start_daemon",
            i18n::tr(lang, "tray.startDaemon").to_string(),
            true,
        ));
    }
    nodes
}

/// agent 家族展示名（托盘 Agent 子菜单标签用；与 `AgentKind::label` 同口径）。
fn agent_kind_label(kind: &str) -> &str {
    match crate::agents::AgentKind::parse(kind) {
        Some(k) => k.label(),
        None => kind,
    }
}

/// 托盘 Agent 条目的状态前缀；复用 /watch 卡片的状态文案与圆点。
fn agent_state_label(state: &str, lang: Lang) -> &'static str {
    match state {
        "working" => i18n::tr(lang, "watch.stateWorking"),
        _ => i18n::tr(lang, "watch.stateIdle"),
    }
}

/// Agent 子菜单条目里会话标题的截断长度（与「待答」预览同 24 字符口径）。
const AGENT_TITLE_MAX_CHARS: usize = 24;

/// 按 Unicode 字符截断，超出追加省略号（与 `daemon::request::truncate_chars` 同逻辑）。
fn truncate_chars(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// 托盘菜单事件分派（由 launch() 的全局 `on_menu_event` 在宿主进程中调用）。
pub fn on_menu_event(app: &AppHandle, id: &str) {
    // 待答子菜单项：聚焦对应弹窗（向 daemon 发 FocusRequest，由 daemon 转发到该请求的弹窗进程）。
    if let Some(request_id) = id.strip_prefix("focus_req:") {
        let request_id = request_id.to_string();
        tauri::async_runtime::spawn(async move {
            if let Ok(stream) = transport::connect().await {
                let (_r, mut w) = stream.into_split();
                let _ = ipc::write_msg(&mut w, &ClientMsg::FocusRequest { request_id }).await;
            }
        });
        return;
    }
    // Agent 子菜单「发送消息」：宿主本进程直接开（或聚焦）该 session 的插话 composer 窗口。
    if let Some(session_id) = id.strip_prefix("ij:") {
        let info = app.try_state::<HostState>().and_then(|s| {
            s.data
                .lock()
                .unwrap()
                .agents
                .iter()
                .find(|a| a.session_id == session_id)
                .cloned()
        });
        if let Some(a) = info {
            open_window(
                app,
                WindowKind::Interject,
                false,
                None,
                Some(crate::gui_host::InterjectTarget {
                    session: a.session_id,
                    agent: Some(a.kind),
                    cwd: a.cwd,
                }),
            );
        }
        return;
    }
    // Agent 子菜单「要求提问」：追加一条固定插话，不打开 composer、不覆盖已有待送达内容。
    if let Some(session_id) = id.strip_prefix("ijask:") {
        let session_id = session_id.to_string();
        tauri::async_runtime::spawn(async move {
            if let Ok(stream) = transport::connect().await {
                let (_r, mut w) = stream.into_split();
                let _ = ipc::write_msg(
                    &mut w,
                    &ClientMsg::InterjectAppend {
                        session_id,
                        text: QUICK_ASK_INTERJECT.to_string(),
                    },
                )
                .await;
            }
        });
        return;
    }
    // Agent 子菜单「聚焦终端」：AppleScript 可能阻塞（授权弹窗等），放后台线程。
    if let Some(session_id) = id.strip_prefix("term:") {
        let pid = app.try_state::<HostState>().and_then(|s| {
            s.data
                .lock()
                .unwrap()
                .agents
                .iter()
                .find(|a| a.session_id == session_id)
                .and_then(|a| a.pid)
        });
        if let Some(pid) = pid {
            std::thread::spawn(move || {
                let _ = crate::integrations::terminal_focus::focus_agent_terminal(pid);
            });
        }
        return;
    }
    match id {
        "open_settings" => open_window(app, WindowKind::Settings, false, None, None),
        // 托盘「历史」无调用方项目上下文 → 默认展示全部项目。
        "open_history" => open_window(app, WindowKind::History, true, None, None),
        "open_agents" => open_window(app, WindowKind::Agents, false, None, None),
        "check_update" => {
            tauri::async_runtime::spawn(async {
                if let Ok(info) = crate::update::check().await {
                    crate::update::state::record_check(&info.latest_version, &info.release_notes);
                }
            });
        }
        "apply_update" => {
            tauri::async_runtime::spawn(async {
                let updater = crate::update::select_updater();
                if updater.apply(None).await.is_ok() {
                    crate::update::state::set_pending(true);
                }
            });
        }
        "start_daemon" => {
            tauri::async_runtime::spawn(async {
                let _ = crate::client::ensure_running().await;
            });
        }
        "restart_daemon" => {
            tauri::async_runtime::spawn(async {
                let _ = crate::client::request_stop(false).await;
                crate::client::wait_until_down(Duration::from_secs(5)).await;
                let _ = crate::client::ensure_running().await;
            });
        }
        "stop_daemon" => {
            tauri::async_runtime::spawn(async {
                let _ = crate::client::request_stop(false).await;
            });
        }
        _ => {}
    }
}

// ===== 窗口管理 =====

/// 在宿主进程内打开（或聚焦）指定窗口，并刷新窗口计数 / 续命。须在主线程调用。
/// `project` 仅历史窗口使用：携带调用方项目 key（默认过滤到该项目），None 则用宿主自身项目。
/// `target` 仅插话窗口使用（session 必填；缺失则忽略本次请求）。
pub(crate) fn open_window(
    app: &AppHandle,
    kind: WindowKind,
    all: bool,
    project: Option<String>,
    target: Option<crate::gui_host::InterjectTarget>,
) {
    let cfg = AppConfig::load_without_secrets();
    // 弹窗在「另一个进程」（daemon 拉起的助手），宿主无 popup 窗口可探测；改据 daemon 在途请求数
    // 判定：置顶开启且有在途请求（即有弹窗在屏）→ 让设置/历史与弹窗同级，浮于其上。
    let pin_above_popup = cfg.general.always_on_top
        && app
            .try_state::<HostState>()
            .map(|s| s.data.lock().unwrap().active_requests > 0)
            .unwrap_or(false);
    let r = match kind {
        WindowKind::Settings => crate::app::create_settings_window(app, &cfg, pin_above_popup),
        WindowKind::History => {
            crate::app::create_history_window(app, &cfg, all, project.as_deref(), pin_above_popup)
        }
        WindowKind::Agents => crate::app::create_agents_window(app, &cfg),
        WindowKind::Interject => match &target {
            Some(t) => crate::app::create_interject_window(app, &cfg, t, pin_above_popup),
            None => return, // session 缺失：无法定位目标 agent，忽略。
        },
    };
    if r.is_ok() {
        // 宿主是 accessory app（不自动激活）：新建窗口需显式聚焦，才能前置到置顶弹窗之上并接收键盘。
        let label = match kind {
            WindowKind::Settings => "settings".to_string(),
            WindowKind::History => "history".to_string(),
            WindowKind::Agents => "agents".to_string(),
            WindowKind::Interject => target
                .as_ref()
                .map(|t| crate::gui_host::interject_label(&t.session))
                .unwrap_or_default(),
        };
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.set_focus();
        }
        // Agent 订阅**不在此处启动**：必须等前端注册好 `agents-updated` 监听后再经命令触发
        // （`start_agents_subscription` → `restart_agents_subscription`），否则 daemon 的首帧
        // 立即快照会早于监听而丢失，导致窗口长时间停在 Loading。
    }
    recount_windows(app);
}

/// 重算宿主承载的窗口数，并据此维护续命连接与退出判定。可在任意线程调用（只读窗口表 + 原子）。
pub fn recount_windows(app: &AppHandle) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    let n = app
        .webview_windows()
        .keys()
        .filter(|l| is_hosted_label(l))
        .count();
    state.windows_open.store(n, Ordering::SeqCst);
    if n > 0 {
        state.ever_open.store(true, Ordering::SeqCst);
    }
    // Agent 窗口已关 → 停订阅（释放 daemon 连接，避免长命宿主借订阅一直把 daemon 续命）。
    if app.get_webview_window("agents").is_none() {
        stop_agents_subscription(app);
    }
    update_keepalive(app);
    evaluate_exit(app);
}

/// （重）启动 agent 状态订阅：停掉旧的，再开一条新订阅。由前端在监听就绪后经命令触发——重启可让
/// daemon 重推一帧立即快照，从而每次开窗都能立刻拿到数据（长命宿主复用旧订阅则会首屏长 Loading）。
pub fn restart_agents_subscription(app: &AppHandle) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    let mut slot = state.agents_sub.lock().unwrap();
    if let Some(old) = slot.take() {
        old.notify_one(); // 停旧订阅任务
    }
    let stop = Arc::new(Notify::new());
    *slot = Some(stop.clone());
    crate::app::spawn_agents_subscription(app.clone(), Some(stop));
}

/// 停掉 agent 状态订阅（agent 窗口关闭时调用）。
pub fn stop_agents_subscription(app: &AppHandle) {
    if let Some(state) = app.try_state::<HostState>() {
        if let Some(old) = state.agents_sub.lock().unwrap().take() {
            old.notify_one();
        }
    }
}

/// 维护「窗口期续命连接」（spec D5）：有窗口且 daemon 在跑 → 持一条普通连接（计入 daemon active）；
/// 无窗口或 daemon 不在 → 关闭。
fn update_keepalive(app: &AppHandle) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    let windows = state.windows_open.load(Ordering::SeqCst);
    let up = state.daemon_up.load(Ordering::SeqCst);
    let mut ka = state.keepalive.lock().unwrap();
    if windows > 0 && up && ka.is_none() {
        let stop = Arc::new(Notify::new());
        *ka = Some(stop.clone());
        tauri::async_runtime::spawn(keepalive_task(stop));
    } else if (windows == 0 || !up) && ka.is_some() {
        if let Some(stop) = ka.take() {
            stop.notify_one();
        }
    }
}

/// 续命连接任务：开一条普通连接给 daemon（不发消息，纯占位计入 active），直到被通知停止或 daemon 断开。
async fn keepalive_task(stop: Arc<Notify>) {
    // 仅给「正在运行的」daemon 续命：用普通 connect（不 ensure_running），连不上即不续命。
    if let Ok(stream) = transport::connect().await {
        let (r, _w) = stream.into_split();
        let mut reader = BufReader::new(r);
        tokio::select! {
            _ = stop.notified() => {}
            _ = async {
                // daemon 主动断开（如换新退出）→ 读到 EOF 即结束。
                loop {
                    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                        Ok(Some(_)) => continue,
                        _ => break,
                    }
                }
            } => {}
        }
        // stream 在此 drop → 连接关闭 → daemon active -= 1（重新计时空闲退出）。
    }
}

/// 宿主退出判定（spec D4/D5/§5.4）。可在任意线程调用。
fn evaluate_exit(app: &AppHandle) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    if state.windows_open.load(Ordering::SeqCst) > 0 {
        return; // 有窗口绝不退出。
    }
    let settled = state.ever_open.load(Ordering::SeqCst) || state.grace_over.load(Ordering::SeqCst);
    match state.mode() {
        // off：宿主仅为窗口而生，窗口都关了即退出。
        MenuBarIconMode::Off => {
            if settled {
                app.exit(0);
            }
        }
        // active：daemon 断连且无窗口 → 图标消失、宿主退出。
        MenuBarIconMode::Active => {
            if settled && !state.daemon_up.load(Ordering::SeqCst) {
                app.exit(0);
            }
        }
        // always：常驻（图标转停止态，不退出）。
        MenuBarIconMode::Always => {}
    }
}

// ===== 自有 IPC 监听（gui-host.sock）=====

fn start_ipc_listener(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let listener = match crate::gui_host::bind() {
            Ok(l) => l,
            Err(_) => return,
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(handle_host_conn(stream, app));
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    });
}

async fn handle_host_conn(stream: tokio::net::UnixStream, app: AppHandle) {
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    while let Ok(Some(msg)) = ipc::read_msg::<_, HostMsg>(&mut reader).await {
        match msg {
            HostMsg::OpenWindow {
                kind,
                all,
                project,
                session,
                agent,
                cwd,
            } => {
                // 回执（让客户端确认已受理），再到主线程开窗。
                let _ = ipc::write_msg(&mut w, &HostMsg::Ping).await;
                let target = session.map(|session| crate::gui_host::InterjectTarget {
                    session,
                    agent,
                    cwd,
                });
                let app2 = app.clone();
                let _ =
                    app.run_on_main_thread(move || open_window(&app2, kind, all, project, target));
            }
            HostMsg::Ping => {
                let _ = ipc::write_msg(&mut w, &HostMsg::Ping).await;
            }
            HostMsg::Shutdown => {
                let app2 = app.clone();
                let _ = app.run_on_main_thread(move || app2.exit(0));
                return;
            }
        }
    }
}

// ===== daemon 状态订阅（非保活）=====

fn start_status_subscription(app: AppHandle) {
    // 事件驱动重连信号：daemon socket 出现/变化（daemon 起停）即唤醒下方循环立即重连，
    // 取代「daemon 关着时每 2s 盲连」的忙轮询。配 30s 兜底超时防漏事件。
    let sock_event = Arc::new(Notify::new());
    spawn_daemon_sock_watch(sock_event.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            // off 模式无托盘，不必订阅；等 socket 事件或 2s 复查模式（覆盖运行时切到 active/always）。
            if mode_of(&app) == MenuBarIconMode::Off {
                tokio::select! {
                    _ = sock_event.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
                continue;
            }
            match transport::connect().await {
                Ok(stream) => {
                    let (r, mut w) = stream.into_split();
                    let mut reader = BufReader::new(r);
                    if ipc::write_msg(&mut w, &ClientMsg::TraySubscribe)
                        .await
                        .is_ok()
                    {
                        set_daemon_up(&app, true);
                        loop {
                            match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
                                Ok(Some(ServerMsg::TrayState {
                                    running,
                                    version,
                                    uptime_secs,
                                    active_requests,
                                    im_connections,
                                    draining,
                                    agents_working,
                                    agents_idle,
                                    update_available,
                                    update_latest,
                                    pending,
                                    pending_requests,
                                    agents,
                                })) => {
                                    if let Some(state) = app.try_state::<HostState>() {
                                        *state.data.lock().unwrap() = TrayData {
                                            running,
                                            version,
                                            uptime_secs,
                                            active_requests,
                                            im_connections,
                                            draining,
                                            agents_working,
                                            agents_idle,
                                            update_available,
                                            update_latest,
                                            pending,
                                            pending_requests,
                                            agents,
                                        };
                                    }
                                    refresh_on_main(&app);
                                    maybe_refresh_binary(&app);
                                }
                                Ok(Some(_)) => {} // 忽略未知 / 其它变体（兼容）。
                                Ok(None) | Err(_) => break,
                            }
                        }
                    }
                }
                Err(_) => {}
            }
            // 断连：daemon 空闲退出 / 停止 / 换新。刷新会因签名不变而仅在「运行→停止」首刷生效。
            set_daemon_up(&app, false);
            refresh_on_main(&app);
            evaluate_exit(&app);
            // 事件驱动重连：等 daemon socket 出现/变化即重连，30s 兜底防漏事件（取代 2s 忙轮询）。
            tokio::select! {
                _ = sock_event.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    });
}

/// 监听 daemon socket（`~/.askhuman/daemon.sock`）所在目录：文件创建/变化（daemon 起停）即唤醒
/// 状态订阅循环立即重连。用一条 `Notify` 跨「notify 同步回调线程」与「异步订阅循环」传递信号。
fn spawn_daemon_sock_watch(event: Arc<Notify>) {
    std::thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};
        use std::sync::mpsc::channel;
        let sock = crate::ipc::transport::socket_path();
        let Some(name) = sock.file_name().map(|n| n.to_os_string()) else {
            return;
        };
        let Some(dir) = sock.parent().map(|d| d.to_path_buf()) else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let (tx, rx) = channel::<()>();
        let mut watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(ev) = res {
                    if ev
                        .paths
                        .iter()
                        .any(|p| p.file_name() == Some(name.as_os_str()))
                    {
                        let _ = tx.send(());
                    }
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        // 收到事件即唤醒订阅循环（Notify 会暂存一个许可，避免错过唤醒）。
        while rx.recv().is_ok() {
            event.notify_one();
        }
    });
}

fn mode_of(app: &AppHandle) -> MenuBarIconMode {
    app.try_state::<HostState>()
        .map(|s| s.mode())
        .unwrap_or(MenuBarIconMode::Off)
}

fn set_daemon_up(app: &AppHandle, up: bool) {
    if let Some(state) = app.try_state::<HostState>() {
        state.daemon_up.store(up, Ordering::SeqCst);
    }
    update_keepalive(app);
}

fn refresh_on_main(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || refresh_tray(&app2));
}

// ===== 配置监听（模式 / 语言 / 登录项）=====

fn start_config_watch(app: AppHandle) {
    std::thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};
        use std::sync::mpsc::{channel, RecvTimeoutError};
        let dir = crate::paths::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let (tx, rx) = channel::<()>();
        let mut watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(ev) = res {
                    let hit = ev
                        .paths
                        .iter()
                        .any(|p| p.file_name().map(|n| n == "config.json").unwrap_or(false));
                    if hit {
                        let _ = tx.send(());
                    }
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        loop {
            if rx.recv().is_err() {
                break;
            }
            // 去抖：合并连续事件。
            loop {
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(()) => continue,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            let cfg = AppConfig::load_without_secrets();
            let app2 = app.clone();
            if app
                .run_on_main_thread(move || apply_config(&app2, &cfg))
                .is_err()
            {
                break; // app 已退出。
            }
        }
    });
}

/// 应用新配置（主线程）：语言热切换、模式切换（建/移图标 + 登录项 + 活动策略）。
fn apply_config(app: &AppHandle, cfg: &AppConfig) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    let new_lang = Lang::resolve(&cfg.general.language);
    let new_mode = cfg.general.menu_bar_icon;
    let old_mode = state.mode();
    *state.lang.lock().unwrap() = new_lang;
    *state.mode.lock().unwrap() = new_mode;

    if new_mode != old_mode {
        apply_activation_policy(app, new_mode);
        match new_mode {
            MenuBarIconMode::Off => {
                let _ = ensure_tray(app, false);
                let _ = crate::integrations::login_item::uninstall();
                evaluate_exit(app);
            }
            MenuBarIconMode::Active => {
                let _ = crate::integrations::login_item::uninstall();
                if tray_supported() {
                    let _ = ensure_tray(app, true);
                }
            }
            MenuBarIconMode::Always => {
                let _ = crate::integrations::login_item::ensure_installed();
                if tray_supported() {
                    let _ = ensure_tray(app, true);
                }
            }
        }
    }

    // 守护进程生命周期换挡：切到「保活」即视为一次触发——立即拉起 daemon（若未运行）。
    // 登录项（开机自启）由 daemon 自身在启动 / on_config_changed 时同步（见 daemon::sync_daemon_login_item），
    // 宿主这里只负责「立即起」，避免开了开关却看不到效果。关掉保活不强杀：让 daemon 按原空闲策略自然退出。
    let new_life = cfg.general.daemon_lifecycle;
    let old_life = *state.daemon_lifecycle.lock().unwrap();
    *state.daemon_lifecycle.lock().unwrap() = new_life;
    if new_life != old_life && new_life == DaemonLifecycleMode::KeepAlive {
        tauri::async_runtime::spawn(async {
            let _ = crate::client::ensure_running().await;
        });
    }

    // 语言 / 状态变化即重建菜单。
    refresh_tray(app);
}

// ===== 二进制换新（spec D11）=====

fn start_binary_watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            maybe_refresh_binary(&app);
        }
    });
}

/// 盘上二进制内容变化且无打开窗口 → 换到新版（不打断在用窗口）。
fn maybe_refresh_binary(app: &AppHandle) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    if state.windows_open.load(Ordering::SeqCst) > 0 {
        return;
    }
    // 以「自身盘上二进制内容是否变化」为准（pending 仅作提示）；新实例会捕获新指纹，不会循环。
    if lifecycle::current_fingerprint() == state.startup_fp {
        return;
    }
    let mode = state.mode();
    release_lock(); // 让新实例可抢锁。
    #[cfg(target_os = "macos")]
    {
        // always：交 launchd KeepAlive 用新二进制重启；其它：自我 re-exec。
        if mode == MenuBarIconMode::Always {
            app.exit(0);
            return;
        }
    }
    let _ = crate::gui_host::spawn_detached();
    let _ = mode;
    app.exit(0);
}
