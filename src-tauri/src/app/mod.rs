//! Tauri 运行时：创建窗口、并行启动 Channel、汇集结果并退出。

pub mod coordinator;

use crate::channels::dingding::DingTalkChannel;
use crate::channels::popup::PopupChannel;
use crate::channels::telegram::TelegramChannel;
use crate::channels::Channel;
use crate::cli::{image_writer, output};
use crate::config::{AppConfig, ThemeMode, WindowEffect};
use crate::dingtalk::client::DingTalkClient;
use crate::models::{AskRequest, ChannelAction, ChannelResult};
use crate::telegram::TelegramClient;
use coordinator::Coordinator;
use std::sync::Arc;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
#[cfg(target_os = "macos")]
use tauri::window::{Effect, EffectState, EffectsBuilder};

/// 运行时只读状态：供 popup_init 拉取请求内容与主题。
pub struct AppState {
    pub request: AskRequest,
    pub config: AppConfig,
}

#[derive(Clone, Copy)]
enum View {
    Popup,
    Settings,
}

/// 无任何可用通信 Channel 时的退出码（供下游据此降级）。
pub const EXIT_NO_CHANNEL: i32 = 3;

/// 提问模式入口：按 Channel 可用性分流到 GUI 弹窗或 headless 消息渠道。
///
/// 决策（在创建任何窗口前）：
/// - 需要弹窗且 GUI 可用 → GUI 路径（弹窗 + 可选会话型渠道抢答）；
/// - 否则若存在可用会话型渠道（Telegram/钉钉）→ headless 路径（不进 Tauri）；
/// - 都不可用 → stderr 报原因 + 退出码 `EXIT_NO_CHANNEL`。
pub fn run_ask(request: AskRequest, config: AppConfig) -> ! {
    let messaging_active = has_active_messaging(&config);
    let popup_wanted = config.channels.popup.enabled;
    let gui = gui_available();

    if popup_wanted && gui.is_ok() {
        run_gui_ask(request, config, messaging_active);
    } else if messaging_active {
        if popup_wanted {
            if let Err(reason) = &gui {
                stderr_redirect::eprintln_real(&format!(
                    "本地弹窗不可用：{}；已改用消息渠道",
                    reason
                ));
            }
        }
        run_headless(request, config);
    } else {
        let reason = match (popup_wanted, &gui) {
            (true, Err(r)) => format!("本地弹窗不可用：{}，且未配置可用的消息渠道", r),
            (false, _) => "本地弹窗已禁用，且未配置可用的消息渠道".to_string(),
            (true, Ok(())) => unreachable!(),
        };
        stderr_redirect::eprintln_real(&format!("错误: 无可用的通信 Channel — {}", reason));
        std::process::exit(EXIT_NO_CHANNEL);
    }
}

/// Telegram 是否已配置且可用（构造 client 成功即视为可用）。
fn is_telegram_active(config: &AppConfig) -> bool {
    let tg = &config.channels.telegram;
    tg.enabled
        && TelegramClient::new(tg.bot_token.clone(), tg.chat_id.clone(), tg.api_base_url.clone())
            .is_ok()
}

/// 钉钉是否已配置且可用（构造 client 成功——即三项非空——即视为可用）。
fn is_dingding_active(config: &AppConfig) -> bool {
    let dd = &config.channels.dingding;
    dd.enabled && DingTalkClient::new(dd).is_ok()
}

/// 是否存在任一可用的会话型消息渠道。
fn has_active_messaging(config: &AppConfig) -> bool {
    is_telegram_active(config) || is_dingding_active(config)
}

/// 收集全部可用的会话型渠道外层（供 GUI 路径注册并行抢答）。
fn active_messaging_channels(config: &AppConfig) -> Vec<Arc<dyn Channel>> {
    let mut channels: Vec<Arc<dyn Channel>> = Vec::new();
    if is_telegram_active(config) {
        channels.push(Arc::new(TelegramChannel::new(config.channels.telegram.clone())));
    }
    if is_dingding_active(config) {
        channels.push(Arc::new(DingTalkChannel::new(config.channels.dingding.clone())));
    }
    channels
}

/// GUI 弹窗路径；若 Tauri 构建失败（GUI 不可用），按消息渠道是否可用兜底。
fn run_gui_ask(request: AskRequest, config: AppConfig, messaging_active: bool) -> ! {
    let state = AppState {
        request: request.clone(),
        config: config.clone(),
    };
    match launch(state, View::Popup) {
        Ok(()) => std::process::exit(0), // 成功路径已在 launch 内退出，此处不可达
        Err(e) => {
            if messaging_active {
                stderr_redirect::eprintln_real(&format!(
                    "本地弹窗启动失败：{}；已改用消息渠道",
                    e
                ));
                run_headless(request, config);
            } else {
                stderr_redirect::eprintln_real(&format!(
                    "错误: 本地弹窗启动失败：{}，且未配置可用的消息渠道",
                    e
                ));
                std::process::exit(EXIT_NO_CHANNEL);
            }
        }
    }
}

/// headless 路径：不进入 Tauri 事件循环，用 tokio 并行跑全部可用会话型渠道。
///
/// 直接驱动各渠道会话并 `await` 全部结束：任一渠道完成回复即 `submit` → `process::exit`；
/// 全部会话结束仍无结果 → 报错并以 `EXIT_NO_CHANNEL` 退出（避免静默挂起）。
fn run_headless(request: AskRequest, config: AppConfig) -> ! {
    use crate::channels::conversation::{run_conversation, MessagingChannel};

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            stderr_redirect::eprintln_real(&format!("错误: 无法创建运行时: {}", e));
            std::process::exit(1);
        }
    };

    let coordinator = Coordinator::new_headless(request.clone());

    rt.block_on(async move {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut handles = Vec::new();

        if is_telegram_active(&config) {
            use crate::channels::telegram::TelegramSession;
            let cfg = config.channels.telegram.clone();
            let req = request.clone();
            let sink = coordinator.clone();
            let cancelled = cancelled.clone();
            handles.push(tokio::spawn(async move {
                let mut session = TelegramSession::new(cfg);
                if let Err(e) = session.open().await {
                    stderr_redirect::eprintln_real(&format!("警告: Telegram 配置无效: {}", e));
                    return;
                }
                run_conversation(&mut session, &req, cancelled, sink).await;
            }));
        }

        if is_dingding_active(&config) {
            use crate::channels::dingding::DingTalkSession;
            let cfg = config.channels.dingding.clone();
            let req = request.clone();
            let sink = coordinator.clone();
            let cancelled = cancelled.clone();
            handles.push(tokio::spawn(async move {
                let mut session = DingTalkSession::new(cfg);
                if let Err(e) = session.open().await {
                    stderr_redirect::eprintln_real(&format!("警告: 钉钉配置无效: {}", e));
                    return;
                }
                run_conversation(&mut session, &req, cancelled, sink).await;
            }));
        }

        for h in handles {
            let _ = h.await;
        }
    });

    // 正常情况下用户完成回复 → submit → 进程已退出；走到此处说明全部会话结束仍未获结果。
    stderr_redirect::eprintln_real("错误: 消息渠道会话结束但未获得结果");
    std::process::exit(EXIT_NO_CHANNEL);
}

/// 设置模式：创建设置窗口。
pub fn run_settings(config: AppConfig) -> ! {
    let state = AppState {
        request: AskRequest::new(crate::models::MessagePrompt::default(), Vec::new(), false),
        config,
    };
    if let Err(e) = launch(state, View::Settings) {
        stderr_redirect::eprintln_real(&format!("错误: 无法启动设置界面: {}", e));
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// 统一启动入口：`generate_context!` 每个二进制只能展开一次，故所有窗口共用此路径。
/// 成功路径在内部进入事件循环并退出进程（不返回）；构建失败返回 `Err` 供调用方兜底。
fn launch(state: AppState, view: View) -> tauri::Result<()> {
    let theme = window_theme(&state.config);
    let window_bg = background_for(resolved_theme(&state.config));
    let popup_w = state.config.channels.popup.width;
    let popup_h = state.config.channels.popup.height;
    let always_on_top = state.config.general.always_on_top;
    let window_effect = state.config.general.window_effect;
    #[cfg(target_os = "macos")]
    let appear_behavior = state.config.general.appear_animation.ns_animation_behavior();

    // 通道启用判定（仅提问模式使用）。
    let messaging_active = has_active_messaging(&state.config);
    // 弹窗禁用且无可用消息渠道时，兜底仍开弹窗，避免无任何 Channel 导致进程挂起。
    let show_popup = state.config.channels.popup.enabled || !messaging_active;

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            crate::commands::popup_init,
            crate::commands::submit_popup,
            crate::commands::cancel_popup,
            crate::commands::open_path,
            crate::commands::preview_attachments,
            crate::commands::close_preview,
            crate::commands::read_image_data_url,
            crate::commands::file_icon_data_url,
            crate::commands::show_attachment_menu,
            crate::commands::get_settings,
            crate::commands::save_settings,
            crate::commands::get_prompt,
            crate::commands::open_test_popup,
            crate::commands::set_theme,
            crate::commands::update_theme,
            crate::commands::open_settings,
            crate::commands::apply_window_effect,
            crate::commands::start_speech,
            crate::commands::stop_speech,
            crate::commands::flush_speech,
            crate::commands::speech_available,
            crate::commands::cursor_hook_status,
            crate::commands::cursor_hook_install,
            crate::commands::cursor_hook_uninstall,
            crate::commands::cursor_hook_reveal,
            crate::commands::telegram_test,
            crate::commands::dingtalk_test,
            crate::commands::dingtalk_detect_prepare,
            crate::commands::dingtalk_detect_wait,
        ])
        .on_window_event(|window, event| {
            // 仅弹窗参与“关闭即取消 / 记忆尺寸”；设置窗口走默认关闭行为。
            if window.label() != "popup" {
                return;
            }
            match event {
                WindowEvent::CloseRequested { .. } => {
                    if let Some(c) = window.app_handle().try_state::<Arc<Coordinator>>() {
                        c.submit(ChannelResult::cancel("popup"));
                    }
                }
                WindowEvent::Resized(_) => persist_popup_size(window),
                _ => {}
            }
        })
        .setup(move |app| {
            // 裸二进制运行时 Dock 不会用 bundle 图标；运行时显式覆盖（仅影响本进程）。
            #[cfg(target_os = "macos")]
            crate::macos_dock_icon::set_dock_icon();
            match view {
                View::Popup => {
                    let request = app.state::<AppState>().request.clone();
                    // Dock 跳动 + 角标提问数（仅 popup）。
                    #[cfg(target_os = "macos")]
                    crate::macos_dock_icon::announce_questions(request.questions.len());
                    let coordinator = Coordinator::new(app.handle().clone(), request.clone());

                    if show_popup {
                        let builder = WebviewWindowBuilder::new(
                            app,
                            "popup",
                            WebviewUrl::App("index.html?view=popup".into()),
                        )
                        .title("HumanInLoop")
                        .inner_size(popup_w, popup_h)
                        .min_inner_size(420.0, 480.0)
                        .center()
                        // 先隐藏构建，设好原生出现动画后再显示，触发 macOS 窗口出现动画。
                        .visible(false)
                        .always_on_top(always_on_top)
                        .theme(theme);
                        let win = apply_surface(builder, window_bg, window_effect).build()?;
                        // macOS：隐藏构建后先设原生出现动画（样式由设置决定），再 show()。
                        #[cfg(target_os = "macos")]
                        if let Ok(ns) = win.ns_window() {
                            crate::macos_window_anim::set_appear_animation(ns, appear_behavior);
                        }
                        // 玻璃模式：显示前挂整窗 Liquid Glass（旧系统由插件回退 vibrancy）。
                        // 模糊模式：背景已在 apply_surface 构建期挂好，无需处理。
                        #[cfg(target_os = "macos")]
                        if matches!(window_effect, WindowEffect::Glass) {
                            apply_liquid_glass(&win);
                        }
                        let _ = win.show();
                        coordinator.register(Arc::new(PopupChannel::new(app.handle().clone())));
                    }

                    let config = app.state::<AppState>().config.clone();
                    for ch in active_messaging_channels(&config) {
                        coordinator.register(ch.clone());
                        ch.start(&request, coordinator.clone());
                    }

                    app.manage(coordinator);
                }
                View::Settings => {
                    let config = AppConfig::load();
                    create_settings_window(app, &config)?;
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())?;

    // 构建成功后、进入事件循环前静默系统噪音日志（如 macOS 的 TSM CapsLock 日志）。
    stderr_redirect::silence();
    app.run(|_app_handle, _event| {});
    std::process::exit(0);
}

/// 把结果输出到 stdout，返回退出码。供协调器调用。
pub(crate) fn emit_result(request_id: &str, result: &ChannelResult) -> i32 {
    match result.action {
        ChannelAction::Cancel => {
            println!("{}", output::cancel_output());
            0
        }
        ChannelAction::Send => {
            // 逐题落盘图片（按题分子目录避免文件名冲突），再聚合输出。
            let mut image_paths_per_q: Vec<Vec<String>> = Vec::with_capacity(result.answers.len());
            for (i, answer) in result.answers.iter().enumerate() {
                match image_writer::save(&answer.images, request_id, i) {
                    Ok(paths) => image_paths_per_q.push(paths),
                    Err(e) => {
                        stderr_redirect::eprintln_real(&format!("错误: {}", e));
                        return 1;
                    }
                }
            }

            let rendered: Vec<output::RenderedAnswer> = result
                .answers
                .iter()
                .enumerate()
                .map(|(i, answer)| output::RenderedAnswer {
                    selected_options: &answer.selected_options,
                    user_input: answer.user_input.as_deref(),
                    image_paths: &image_paths_per_q[i],
                    file_paths: &answer.files,
                })
                .collect();

            println!("{}", output::aggregate_output(&rendered));
            0
        }
    }
}

/// 解析“实际”主题：system 时探测系统深/浅色。
fn resolved_theme(config: &AppConfig) -> tauri::Theme {
    match config.general.theme {
        ThemeMode::Light => tauri::Theme::Light,
        ThemeMode::Dark => tauri::Theme::Dark,
        ThemeMode::System => match dark_light::detect() {
            Ok(dark_light::Mode::Dark) => tauri::Theme::Dark,
            _ => tauri::Theme::Light,
        },
    }
}

/// 平台相关窗口表面：
/// - macOS：透明窗口 + `underWindowBackground` 毛玻璃（vibrancy），底色由材质提供；
/// - 其它平台：纯色不透明底（无毛玻璃）。
fn apply_surface<'a, R, M>(
    builder: WebviewWindowBuilder<'a, R, M>,
    #[allow(unused_variables)] window_bg: tauri::window::Color,
    #[allow(unused_variables)] effect: WindowEffect,
) -> WebviewWindowBuilder<'a, R, M>
where
    R: tauri::Runtime,
    M: Manager<R>,
{
    #[cfg(target_os = "macos")]
    {
        let builder = builder
            .transparent(true)
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
        match effect {
            // 模糊：构建期挂 Tauri 自带 NSVisualEffectView。
            WindowEffect::Blur => builder.effects(
                EffectsBuilder::new()
                    .effect(Effect::UnderWindowBackground)
                    .state(EffectState::FollowsWindowActiveState)
                    .build(),
            ),
            // 玻璃：此处不挂 vibrancy，背景由 `apply_liquid_glass` 在 build 后接管；
            // 否则 vibrancy 会压在玻璃层之上，看到的仍是普通毛玻璃。
            WindowEffect::Glass => builder,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder.background_color(window_bg)
    }
}

/// macOS：给窗口挂上唯一的背景层。
/// - macOS 26+：`NSGlassEffectView`（Liquid Glass 整窗背景）；
/// - 旧系统：插件自动回退到 `NSVisualEffectView`（等价于此前的 vibrancy）。
/// 因 `apply_surface` 已不再挂 Tauri 自带 vibrancy，这里需对所有 macOS 版本生效。
#[cfg(target_os = "macos")]
fn apply_liquid_glass<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use tauri_plugin_liquid_glass::{LiquidGlassConfig, LiquidGlassExt};
    // 整窗背景：cornerRadius 0，由窗口自身圆角裁剪；不加 tint，使用 Regular 材质。
    let _ = window
        .liquid_glass()
        .set_effect(window, LiquidGlassConfig::default());
}

/// 运行时切换窗口背景效果，供设置页「玻璃/模糊」开关实时作用于已打开窗口。
/// 切换前先卸掉另一种背景层，避免玻璃与 vibrancy 叠加。
#[cfg(target_os = "macos")]
pub(crate) fn set_runtime_window_effect<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    effect: WindowEffect,
) {
    use tauri_plugin_liquid_glass::{LiquidGlassConfig, LiquidGlassExt};
    match effect {
        WindowEffect::Glass => {
            // Tauri 的 set_effects(None) 在 macOS 为空实现，需手动移除残留的 vibrancy 视图。
            if let Ok(ns) = window.ns_window() {
                crate::macos_window_anim::remove_vibrancy_views(ns);
            }
            apply_liquid_glass(window);
        }
        WindowEffect::Blur => {
            let _ = window.liquid_glass().set_effect(
                window,
                LiquidGlassConfig {
                    enabled: false,
                    ..Default::default()
                },
            );
            // 先清掉旧的 vibrancy，避免重复点击叠加多层。
            if let Ok(ns) = window.ns_window() {
                crate::macos_window_anim::remove_vibrancy_views(ns);
            }
            let _ = window.set_effects(
                EffectsBuilder::new()
                    .effect(Effect::UnderWindowBackground)
                    .state(EffectState::FollowsWindowActiveState)
                    .build(),
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn set_runtime_window_effect<R: tauri::Runtime>(
    _window: &tauri::WebviewWindow<R>,
    _effect: WindowEffect,
) {
}

/// 创建（或聚焦已存在的）设置窗口。供 `--settings` 启动与弹窗导航栏共用。
pub(crate) fn create_settings_window<R, M>(manager: &M, config: &AppConfig) -> tauri::Result<()>
where
    R: tauri::Runtime,
    M: Manager<R>,
{
    if let Some(w) = manager.get_webview_window("settings") {
        let _ = w.set_focus();
        return Ok(());
    }
    let theme = window_theme(config);
    let window_bg = background_for(resolved_theme(config));
    let builder = WebviewWindowBuilder::new(
        manager,
        "settings",
        WebviewUrl::App("index.html?view=settings".into()),
    )
    .title("HumanInLoop 设置")
    .inner_size(560.0, 640.0)
    // 最小宽度：保证标题栏内居中的 tab 不会与左上角红绿灯重叠。
    .min_inner_size(480.0, 520.0)
    .center()
    .theme(theme);
    let window_effect = config.general.window_effect;
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    let win = apply_surface(builder, window_bg, window_effect).build()?;
    #[cfg(target_os = "macos")]
    if matches!(window_effect, WindowEffect::Glass) {
        apply_liquid_glass(&win);
    }
    Ok(())
}

/// 原生窗口/webview 底色（与前端 tokens.css `--bg` 对齐）。
fn background_for(theme: tauri::Theme) -> tauri::window::Color {
    match theme {
        tauri::Theme::Dark => tauri::window::Color(30, 30, 30, 255),
        _ => tauri::window::Color(255, 255, 255, 255),
    }
}

fn window_theme(config: &AppConfig) -> Option<tauri::Theme> {
    match config.general.theme {
        ThemeMode::Light => Some(tauri::Theme::Light),
        ThemeMode::Dark => Some(tauri::Theme::Dark),
        ThemeMode::System => None,
    }
}

/// 记住窗口尺寸：用户拉伸后把逻辑尺寸写回配置。
fn persist_popup_size(window: &tauri::Window) {
    let state = window.app_handle().state::<AppState>();
    if !state.config.channels.popup.remember_size {
        return;
    }
    if let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) {
        let mut cfg = AppConfig::load();
        cfg.channels.popup.width = size.width as f64 / scale;
        cfg.channels.popup.height = size.height as f64 / scale;
        let _ = cfg.save();
    }
}

/// GUI 是否可用（进入 Tauri 前的轻量预探测）。
///
/// 因 release 为 `panic = "abort"`，无法用 `catch_unwind` 兜住 GUI 初始化崩溃，
/// 故在 Linux 上先探测显示环境与 WebKitGTK；实际 `build()` 失败仍由调用方按 `Err` 兜底。
/// macOS / Windows 使用系统 WebView，默认视为可用。
#[cfg(target_os = "linux")]
fn gui_available() -> Result<(), String> {
    let has_display = std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !has_display {
        return Err("无图形显示环境（DISPLAY / WAYLAND_DISPLAY 均未设置）".to_string());
    }
    if !webkitgtk_loadable() {
        return Err("系统缺少 WebKitGTK（如 libwebkit2gtk-4.1）".to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn gui_available() -> Result<(), String> {
    Ok(())
}

/// 探测 WebKitGTK 运行库是否可被加载（dlopen 成功即视为可用）。
#[cfg(target_os = "linux")]
fn webkitgtk_loadable() -> bool {
    use std::ffi::CString;
    const CANDIDATES: [&str; 4] = [
        "libwebkit2gtk-4.1.so.0",
        "libwebkit2gtk-4.1.so",
        "libwebkit2gtk-4.0.so.37",
        "libwebkit2gtk-4.0.so",
    ];
    for name in CANDIDATES {
        if let Ok(c) = CString::new(name) {
            unsafe {
                let handle = libc::dlopen(c.as_ptr(), libc::RTLD_LAZY);
                if !handle.is_null() {
                    libc::dlclose(handle);
                    return true;
                }
            }
        }
    }
    false
}

/// 静默 GUI 事件循环期间的系统噪音日志：把进程 stderr 重定向到 /dev/null，
/// 同时保存原始 stderr 句柄，供我们自己的错误信息照常输出。
#[cfg(unix)]
mod stderr_redirect {
    use std::sync::atomic::{AtomicI32, Ordering};

    static SAVED: AtomicI32 = AtomicI32::new(-1);

    pub fn silence() {
        unsafe {
            let saved = libc::dup(libc::STDERR_FILENO);
            if saved < 0 {
                return;
            }
            let devnull =
                libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_WRONLY);
            if devnull < 0 {
                libc::close(saved);
                return;
            }
            libc::dup2(devnull, libc::STDERR_FILENO);
            libc::close(devnull);
            SAVED.store(saved, Ordering::SeqCst);
        }
    }

    pub fn eprintln_real(msg: &str) {
        let fd = SAVED.load(Ordering::SeqCst);
        let line = format!("{}\n", msg);
        if fd >= 0 {
            unsafe {
                libc::write(fd, line.as_ptr() as *const libc::c_void, line.len());
            }
        } else {
            eprint!("{}", line);
        }
    }
}

#[cfg(not(unix))]
mod stderr_redirect {
    pub fn silence() {}
    pub fn eprintln_real(msg: &str) {
        eprintln!("{}", msg);
    }
}
