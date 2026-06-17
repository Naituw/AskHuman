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

use crate::config::{AppConfig, MenuBarIconMode};
use crate::daemon::lifecycle::{self, Fingerprint, LockGuard};
use crate::gui_host::{HostMsg, WindowKind};
use crate::i18n::{self, Lang};
use crate::ipc::{self, transport, ClientMsg, ServerMsg};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager};
use tokio::io::BufReader;
use tokio::sync::Notify;

/// 由宿主统一承载的窗口标签（用于窗口计数 / 续命判定）。
const WINDOW_LABELS: [&str; 3] = ["settings", "history", "agents"];

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
}

pub struct HostState {
    pub mode: Mutex<MenuBarIconMode>,
    pub lang: Mutex<Lang>,
    pub data: Mutex<TrayData>,
    pub daemon_up: AtomicBool,
    pub windows_open: AtomicUsize,
    /// 是否曾经打开过窗口（off/active 模式退出判定用，避免开窗前误退）。
    pub ever_open: AtomicBool,
    /// 启动宽限是否已过（覆盖「OpenWindow 始终未到达」的兜底退出）。
    pub grace_over: AtomicBool,
    pub tray: Mutex<Option<TrayIcon>>,
    /// 托盘菜单对象（**长期持有同一个**）：刷新时只原地增删/改其条目，绝不 `set_menu` 换新对象——
    /// 替换 `NSStatusItem.menu` 会把用户正展开的菜单关掉，原地改条目则不会（spec 菜单稳定性）。
    pub menu: Mutex<Option<Menu<tauri::Wry>>>,
    /// 上次渲染的「菜单/图标内容签名」：与本次相同则整次刷新直接跳过，连菜单条目都不动——
    /// daemon 持续停止时内容不变 → 不重建菜单 → 展开的菜单不会被挤掉。
    pub menu_sig: Mutex<Option<String>>,
    /// 窗口期续命连接的停止信号（持有即有续命连接在）。
    pub keepalive: Mutex<Option<Arc<Notify>>>,
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
        lang: Mutex::new(lang),
        data: Mutex::new(TrayData::default()),
        daemon_up: AtomicBool::new(false),
        windows_open: AtomicUsize::new(0),
        ever_open: AtomicBool::new(false),
        grace_over: AtomicBool::new(false),
        tray: Mutex::new(None),
        menu: Mutex::new(None),
        menu_sig: Mutex::new(None),
        keepalive: Mutex::new(None),
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
        *state.menu.lock().unwrap() = None; // 菜单随托盘失效；下次重建时再新建并填充
        *state.menu_sig.lock().unwrap() = None;
        return Ok(());
    }
    if state.tray.lock().unwrap().is_some() {
        refresh_tray(app);
        return Ok(());
    }
    // 建一个空菜单挂上托盘并长期持有；条目交给 refresh_tray 原地填充（之后只原地增删，绝不换对象）。
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
    *state.menu.lock().unwrap() = Some(menu);
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

    // 内容签名：与上次相同 → 整次跳过，不触碰托盘（避免重建菜单把用户正展开的菜单挤掉）。
    let sig = menu_signature(up, lang, &data);
    if state.menu_sig.lock().unwrap().as_deref() == Some(sig.as_str()) {
        return;
    }

    // 图标（set_icon / set_tooltip 不会关闭已展开菜单）。
    if let Some(img) = icon_for(up, data.active_requests) {
        let _ = tray.set_icon(Some(img));
        let _ = tray.set_icon_as_template(icon_bytes::TEMPLATE);
    }
    // 菜单：原地清空 + 重新填充**同一个**菜单对象（不 set_menu 换对象）。
    if let Some(menu) = state.menu.lock().unwrap().as_ref() {
        clear_menu(menu);
        let _ = populate_menu(app, menu, up, lang, &data);
    }
    let tip = if up {
        i18n::tr(lang, "tray.tooltipRunning").to_string()
    } else {
        i18n::tr(lang, "tray.tooltipStopped").to_string()
    };
    let _ = tray.set_tooltip(Some(&tip));

    *state.menu_sig.lock().unwrap() = Some(sig);
}

/// 决定菜单/图标渲染结果的全部输入拼成的签名（含语言、在线态、各状态字段；uptime 取分钟级文案，
/// 避免每次推送都因秒数微变而重建菜单）。相同即无需刷新。
fn menu_signature(up: bool, lang: Lang, data: &TrayData) -> String {
    format!(
        "{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
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
        // update_latest 仅在 update_available 时入签名，避免无更新时的噪声变化触发重建。
        if data.update_available {
            data.update_latest.as_str()
        } else {
            ""
        },
    )
}

/// 清空菜单的全部条目（原地操作同一菜单对象，不替换对象）。
fn clear_menu(menu: &Menu<tauri::Wry>) {
    let count = menu.items().map(|v| v.len()).unwrap_or(0);
    for _ in 0..count {
        let _ = menu.remove_at(0);
    }
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

/// 把状态区（disabled 只读）+ 操作区条目**追加**进给定（已清空的）菜单对象（spec D7）。
/// 写入既有对象而非新建，以便原地更新（不替换 NSMenu → 展开中的菜单不被关掉）。
fn populate_menu(
    app: &AppHandle,
    menu: &Menu<tauri::Wry>,
    up: bool,
    lang: Lang,
    data: &TrayData,
) -> tauri::Result<()> {
    let disabled = |text: String| -> tauri::Result<()> {
        let it = MenuItemBuilder::new(text).enabled(false).build(app)?;
        menu.append(&it)
    };
    let action = |id: &str, text: String| -> tauri::Result<()> {
        let it = MenuItemBuilder::with_id(id, text).build(app)?;
        menu.append(&it)
    };
    let sep = |menu: &Menu<tauri::Wry>| -> tauri::Result<()> {
        let s = PredefinedMenuItem::separator(app)?;
        menu.append(&s)
    };

    // —— 状态区 ——
    let title = if up {
        i18n::tr(lang, "tray.running").to_string()
    } else {
        i18n::tr(lang, "tray.stopped").to_string()
    };
    disabled(title)?;
    if up {
        disabled(i18n::tr(lang, "tray.version").replace("{v}", &data.version))?;
        disabled(i18n::tr(lang, "tray.uptime").replace("{d}", &fmt_uptime(data.uptime_secs)))?;
        if data.draining {
            disabled(i18n::tr(lang, "tray.draining").to_string())?;
        }
        if data.active_requests > 0 {
            disabled(
                i18n::tr(lang, "tray.pendingQuestions")
                    .replace("{n}", &data.active_requests.to_string()),
            )?;
        }
        if data.agents_working + data.agents_idle > 0 {
            disabled(
                i18n::tr(lang, "tray.agents")
                    .replace("{w}", &data.agents_working.to_string())
                    .replace("{i}", &data.agents_idle.to_string()),
            )?;
        }
        if !data.im_connections.is_empty() {
            disabled(
                i18n::tr(lang, "tray.imConnections")
                    .replace("{list}", &data.im_connections.join(", ")),
            )?;
        }
    }
    if data.update_available {
        disabled(i18n::tr(lang, "tray.updateAvailable").replace("{v}", &data.update_latest))?;
    }
    if data.pending {
        disabled(i18n::tr(lang, "tray.updatePending").to_string())?;
    }

    // —— 操作区 ——
    sep(menu)?;
    action("open_settings", i18n::tr(lang, "tray.openSettings").to_string())?;
    action("open_history", i18n::tr(lang, "tray.openHistory").to_string())?;
    action("open_agents", i18n::tr(lang, "tray.openAgents").to_string())?;
    sep(menu)?;
    action("check_update", i18n::tr(lang, "tray.checkUpdate").to_string())?;
    if data.update_available {
        action(
            "apply_update",
            i18n::tr(lang, "tray.applyUpdate").replace("{v}", &data.update_latest),
        )?;
    }
    sep(menu)?;
    if up {
        action("restart_daemon", i18n::tr(lang, "tray.restartDaemon").to_string())?;
        action("stop_daemon", i18n::tr(lang, "tray.stopDaemon").to_string())?;
    } else {
        action("start_daemon", i18n::tr(lang, "tray.startDaemon").to_string())?;
    }
    Ok(())
}

/// 托盘菜单事件分派（由 launch() 的全局 `on_menu_event` 在宿主进程中调用）。
pub fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        "open_settings" => open_window(app, WindowKind::Settings, false, None),
        // 托盘「历史」无调用方项目上下文 → 默认展示全部项目。
        "open_history" => open_window(app, WindowKind::History, true, None),
        "open_agents" => open_window(app, WindowKind::Agents, false, None),
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
fn open_window(app: &AppHandle, kind: WindowKind, all: bool, project: Option<String>) {
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
    };
    if r.is_ok() {
        // 宿主是 accessory app（不自动激活）：新建窗口需显式聚焦，才能前置到置顶弹窗之上并接收键盘。
        let label = match kind {
            WindowKind::Settings => "settings",
            WindowKind::History => "history",
            WindowKind::Agents => "agents",
        };
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.set_focus();
        }
        if kind == WindowKind::Agents {
            // Agent 窗口需 daemon 数据：订阅会按需 ensure_running（唯一允许由窗口启动 daemon 的入口）。
            crate::app::start_agents_subscription(app.clone());
        }
    }
    recount_windows(app);
}

/// 重算宿主承载的窗口数，并据此维护续命连接与退出判定。可在任意线程调用（只读窗口表 + 原子）。
pub fn recount_windows(app: &AppHandle) {
    let Some(state) = app.try_state::<HostState>() else {
        return;
    };
    let n = WINDOW_LABELS
        .iter()
        .filter(|l| app.get_webview_window(l).is_some())
        .count();
    state.windows_open.store(n, Ordering::SeqCst);
    if n > 0 {
        state.ever_open.store(true, Ordering::SeqCst);
    }
    update_keepalive(app);
    evaluate_exit(app);
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
            HostMsg::OpenWindow { kind, all, project } => {
                // 回执（让客户端确认已受理），再到主线程开窗。
                let _ = ipc::write_msg(&mut w, &HostMsg::Ping).await;
                let app2 = app.clone();
                let _ = app
                    .run_on_main_thread(move || open_window(&app2, kind, all, project));
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
