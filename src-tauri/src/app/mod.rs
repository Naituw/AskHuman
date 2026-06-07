//! Tauri 运行时：创建窗口、并行启动 Channel、汇集结果并退出。

pub mod coordinator;

use crate::channels::dingding::DingTalkChannel;
use crate::channels::feishu::FeishuChannel;
use crate::channels::popup::PopupChannel;
use crate::channels::telegram::TelegramChannel;
use crate::channels::Channel;
use crate::cli::{image_writer, output};
use crate::config::{AppConfig, ThemeMode, WindowEffect};
use crate::i18n::{self, Lang};
use crate::dingtalk::client::DingTalkClient;
use crate::feishu::client::FeishuClient;
use crate::models::{AskRequest, ChannelAction, ChannelResult, QuestionAnswer};
use crate::telegram::TelegramClient;
use coordinator::Coordinator;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
#[cfg(target_os = "macos")]
use tauri::window::{Effect, EffectState, EffectsBuilder};

/// 运行时只读状态：供 popup_init 拉取请求内容与主题。
pub struct AppState {
    pub request: AskRequest,
    pub config: AppConfig,
    /// 来源名（弹窗标题「Question from {source}」）。Daemon 模式由调用方上送（A11）；
    /// 设置 / 非 Daemon 回退路径取本进程环境。
    pub source: String,
}

#[derive(Clone, Copy)]
enum View {
    Popup,
    Settings,
}

/// GUI Helper 模式下，弹窗 ↔ Daemon 的 IPC 接线（由 `run_gui_helper` 建好后传入 `launch`）。
pub struct PopupIpc {
    /// 向 Daemon 发送 `answer` 等消息（写任务已在 `run_gui_helper` 中起好）。
    pub gui_tx: tokio::sync::mpsc::UnboundedSender<crate::ipc::ClientMsg>,
    /// Daemon 分配的 request_id（回带在 `answer` 中）。
    pub request_id: String,
    /// 读取 Daemon → GUI 的消息流（cancel / 连接断开）。
    pub reader: std::pin::Pin<Box<dyn tokio::io::AsyncBufRead + Send>>,
}

/// 弹窗作答 → Daemon 的桥：把前端 `submit_popup` / `cancel_popup` 转成 IPC `answer` 发回 Daemon。
/// 仅 GUI Helper 模式存在；单进程（非 unix 回退）路径用 `Coordinator`。
pub struct GuiBridge {
    tx: tokio::sync::mpsc::UnboundedSender<crate::ipc::ClientMsg>,
    request_id: String,
    /// 仅投递一次（发送/取消互斥，去重）。
    done: AtomicBool,
    app: tauri::AppHandle,
}

impl GuiBridge {
    fn terminal(&self, action: ChannelAction, answers: Vec<QuestionAnswer>) {
        if self.done.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(crate::ipc::ClientMsg::Answer {
            request_id: self.request_id.clone(),
            action,
            answers,
        });
        // 即时关窗，视觉上与单进程一致（进程随后由 Daemon 关闭连接 / 安全网驱动退出）。
        if let Some(w) = self.app.get_webview_window("popup") {
            let _ = w.close();
        }
        // 安全网：正常情况下 Daemon 收到答复后关闭连接 → reader EOF → 退出；
        // 万一 Daemon 无响应，到时也主动退出，避免弹窗进程悬挂。
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            app.exit(0);
        });
    }

    /// 提交作答。
    pub fn send_answer(&self, answers: Vec<QuestionAnswer>) {
        self.terminal(ChannelAction::Send, answers);
    }

    /// 取消（关窗 / Cmd+Q）。
    pub fn send_cancel(&self) {
        self.terminal(ChannelAction::Cancel, Vec::new());
    }
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
    let lang = Lang::resolve(&config.general.language);
    let messaging_active = has_active_messaging(&config);
    let popup_wanted = config.channels.popup.enabled;
    let gui = gui_available(lang);

    if popup_wanted && gui.is_ok() {
        run_gui_ask(request, config, messaging_active);
    } else if messaging_active {
        if popup_wanted {
            if let Err(reason) = &gui {
                stderr_redirect::eprintln_real(
                    &i18n::tr(lang, "app.popupUnavailableFellBack").replace("{reason}", reason),
                );
            }
        }
        run_headless(request, config);
    } else {
        let reason = match (popup_wanted, &gui) {
            (true, Err(r)) => i18n::tr(lang, "app.popupUnavailableNoChannel").replace("{reason}", r),
            (false, _) => i18n::tr(lang, "app.popupDisabledNoChannel").to_string(),
            (true, Ok(())) => unreachable!(),
        };
        stderr_redirect::eprintln_real(&format!(
            "{}{}",
            i18n::err_prefix(lang),
            i18n::tr(lang, "app.noChannel").replace("{reason}", &reason)
        ));
        std::process::exit(EXIT_NO_CHANNEL);
    }
}

/// Telegram 是否已配置且可用（构造 client 成功即视为可用）。
pub(crate) fn is_telegram_active(config: &AppConfig) -> bool {
    let tg = &config.channels.telegram;
    tg.enabled
        && TelegramClient::new(tg.bot_token.clone(), tg.chat_id.clone(), tg.api_base_url.clone())
            .is_ok()
}

/// 钉钉是否已配置且可用（构造 client 成功——即三项非空——即视为可用）。
pub(crate) fn is_dingding_active(config: &AppConfig) -> bool {
    let dd = &config.channels.dingding;
    dd.enabled && DingTalkClient::new(dd).is_ok()
}

/// 飞书是否已配置且可用（构造 client 成功且 open_id 非空——即四项齐备——即视为可用）。
pub(crate) fn is_feishu_active(config: &AppConfig) -> bool {
    let fs = &config.channels.feishu;
    fs.enabled
        && !fs.open_id.trim().is_empty()
        && FeishuClient::new(fs).is_ok()
}

/// 是否存在任一可用的会话型消息渠道。
fn has_active_messaging(config: &AppConfig) -> bool {
    is_telegram_active(config) || is_dingding_active(config) || is_feishu_active(config)
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
    if is_feishu_active(config) {
        channels.push(Arc::new(FeishuChannel::new(config.channels.feishu.clone())));
    }
    channels
}

/// GUI 弹窗路径；若 Tauri 构建失败（GUI 不可用），按消息渠道是否可用兜底。
fn run_gui_ask(request: AskRequest, config: AppConfig, messaging_active: bool) -> ! {
    let lang = Lang::resolve(&config.general.language);
    let state = AppState {
        request: request.clone(),
        config: config.clone(),
        source: crate::models::source_name(),
    };
    match launch(state, View::Popup, None) {
        Ok(()) => std::process::exit(0), // 成功路径已在 launch 内退出，此处不可达
        Err(e) => {
            if messaging_active {
                stderr_redirect::eprintln_real(
                    &i18n::tr(lang, "app.popupStartFailedFellBack").replace("{e}", &e.to_string()),
                );
                run_headless(request, config);
            } else {
                stderr_redirect::eprintln_real(&format!(
                    "{}{}",
                    i18n::err_prefix(lang),
                    i18n::tr(lang, "app.popupStartFailedNoChannel").replace("{e}", &e.to_string())
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

    let lang = Lang::resolve(&config.general.language);
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            stderr_redirect::eprintln_real(&format!(
                "{}{}",
                i18n::err_prefix(lang),
                i18n::tr(lang, "app.runtimeCreateFailed").replace("{e}", &e.to_string())
            ));
            std::process::exit(1);
        }
    };

    // 并行消息渠道数（用于抢答收尾计算落败端数）+ 共享抢答信号。
    let messaging_count = is_telegram_active(&config) as usize
        + is_dingding_active(&config) as usize
        + is_feishu_active(&config) as usize;
    let preempt = Arc::new(crate::channels::Preemption::new());
    let coordinator = Coordinator::new_headless(request.clone(), preempt.clone(), messaging_count);

    rt.block_on(async move {
        let mut handles = Vec::new();

        if is_telegram_active(&config) {
            use crate::channels::telegram::TelegramSession;
            use crate::telegram::router::TgRouter;
            let cfg = config.channels.telegram.clone();
            let req = request.clone();
            let sink = coordinator.clone();
            let preempt = preempt.clone();
            handles.push(tokio::spawn(async move {
                // 单进程：每进程起一个仅挂本会话的 Router（统一走 Router 路径，单一 offset）。
                let router = match TgRouter::connect(&cfg).await {
                    Ok(r) => r,
                    Err(e) => {
                        stderr_redirect::eprintln_real(&format!(
                            "{}{}",
                            i18n::warn_prefix(lang),
                            i18n::tr(lang, "app.telegramInvalid").replace("{e}", &e)
                        ));
                        return;
                    }
                };
                let events = router.register();
                let mut session = TelegramSession::new(cfg, events);
                if let Err(e) = session.open().await {
                    stderr_redirect::eprintln_real(&format!(
                        "{}{}",
                        i18n::warn_prefix(lang),
                        i18n::tr(lang, "app.telegramInvalid").replace("{e}", &e.to_string())
                    ));
                    return;
                }
                run_conversation(&mut session, &req, preempt, sink).await;
            }));
        }

        if is_dingding_active(&config) {
            use crate::channels::dingding::DingTalkSession;
            use crate::dingtalk::router::DdRouter;
            let cfg = config.channels.dingding.clone();
            let req = request.clone();
            let sink = coordinator.clone();
            let preempt = preempt.clone();
            handles.push(tokio::spawn(async move {
                // 单进程：每进程起一个仅挂本会话的 Router（统一走 Router 路径）。
                let router =
                    match DdRouter::connect(cfg.client_id.trim(), cfg.client_secret.trim()).await {
                        Ok(r) => r,
                        Err(e) => {
                            stderr_redirect::eprintln_real(&format!(
                                "{}{}",
                                i18n::warn_prefix(lang),
                                i18n::tr(lang, "app.dingtalkInvalid").replace("{e}", &e)
                            ));
                            return;
                        }
                    };
                let events = router.register();
                let mut session = DingTalkSession::new(cfg, events);
                if let Err(e) = session.open().await {
                    stderr_redirect::eprintln_real(&format!(
                        "{}{}",
                        i18n::warn_prefix(lang),
                        i18n::tr(lang, "app.dingtalkInvalid").replace("{e}", &e.to_string())
                    ));
                    return;
                }
                run_conversation(&mut session, &req, preempt, sink).await;
            }));
        }

        if is_feishu_active(&config) {
            use crate::channels::feishu::FeishuSession;
            use crate::feishu::router::FsRouter;
            let cfg = config.channels.feishu.clone();
            let req = request.clone();
            let sink = coordinator.clone();
            let preempt = preempt.clone();
            handles.push(tokio::spawn(async move {
                // 单进程：每进程起一个仅挂本会话的 Router（统一走 Router 路径）。
                let router = match FsRouter::connect(&cfg).await {
                    Ok(r) => r,
                    Err(e) => {
                        stderr_redirect::eprintln_real(&format!(
                            "{}{}",
                            i18n::warn_prefix(lang),
                            i18n::tr(lang, "app.feishuInvalid").replace("{e}", &e)
                        ));
                        return;
                    }
                };
                let events = router.register();
                let mut session = FeishuSession::new(cfg, events);
                if let Err(e) = session.open().await {
                    stderr_redirect::eprintln_real(&format!(
                        "{}{}",
                        i18n::warn_prefix(lang),
                        i18n::tr(lang, "app.feishuInvalid").replace("{e}", &e)
                    ));
                    return;
                }
                run_conversation(&mut session, &req, preempt, sink).await;
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        // 全部会话结束：若已有结果则输出并退出（不返回）；否则返回交由下方兜底报错。
        coordinator.finish();
    });

    // 正常情况下用户完成回复 → submit → 进程已退出；走到此处说明全部会话结束仍未获结果。
    stderr_redirect::eprintln_real(&format!(
        "{}{}",
        i18n::err_prefix(lang),
        i18n::tr(lang, "app.sessionEndedNoResult")
    ));
    std::process::exit(EXIT_NO_CHANNEL);
}

/// 设置模式：创建设置窗口。
pub fn run_settings(config: AppConfig) -> ! {
    let lang = Lang::resolve(&config.general.language);
    let state = AppState {
        request: AskRequest::new(crate::models::MessagePrompt::default(), Vec::new(), false),
        config,
        source: crate::models::source_name(),
    };
    if let Err(e) = launch(state, View::Settings, None) {
        stderr_redirect::eprintln_real(&format!(
            "{}{}",
            i18n::err_prefix(lang),
            i18n::tr(lang, "app.settingsLaunchFailed").replace("{e}", &e.to_string())
        ));
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// GUI Helper 模式入口（`AskHuman --popup --endpoint <sock> --token <tok>`，由 Daemon 拉起）。
///
/// 流程：连 Daemon → 出示一次性 token → 收 `show` → 本进程主线程跑 Tauri 弹窗；
/// 用户作答 / 取消经 IPC `answer` 回 Daemon；收到 `cancel` 或连接断开即退出。
#[cfg(unix)]
pub fn run_gui_helper(_endpoint: String, token: String) -> ! {
    use crate::ipc::{self, transport, ClientMsg, ServerMsg};
    use tokio::io::BufReader;

    // 连接 + 握手 + 读 show（在 Tauri 全局运行时上完成，确保后续读写任务同一 reactor）。
    let connected = tauri::async_runtime::block_on(async move {
        let stream = transport::connect().await?;
        let (r, mut w) = stream.into_split();
        let mut reader = BufReader::new(r);
        ipc::write_msg(&mut w, &ClientMsg::GuiHello { token }).await?;
        loop {
            match ipc::read_msg::<_, ServerMsg>(&mut reader).await? {
                Some(ServerMsg::Show(show)) => return Ok::<_, std::io::Error>((show, reader, w)),
                Some(_) => continue,
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "daemon closed before show",
                    ))
                }
            }
        }
    });

    let (show, reader, writer) = match connected {
        Ok(v) => v,
        Err(e) => {
            stderr_redirect::eprintln_real(&format!("askhuman popup helper: {}", e));
            std::process::exit(3);
        }
    };

    // 写任务：把 answer / 取消等消息串行写回 Daemon。
    let (gui_tx, mut gui_rx) = tokio::sync::mpsc::unbounded_channel::<ClientMsg>();
    tauri::async_runtime::spawn(async move {
        let mut writer = writer;
        while let Some(msg) = gui_rx.recv().await {
            if ipc::write_msg(&mut writer, &msg).await.is_err() {
                break;
            }
        }
    });

    let request_id = show.request_id.clone();
    let state = AppState {
        request: show.request,
        config: AppConfig::load(),
        source: show.source,
    };
    let popup_ipc = PopupIpc {
        gui_tx,
        request_id,
        reader: Box::pin(reader),
    };
    if let Err(e) = launch(state, View::Popup, Some(popup_ipc)) {
        stderr_redirect::eprintln_real(&format!("askhuman popup helper failed: {}", e));
        std::process::exit(3);
    }
    std::process::exit(0);
}

/// 统一启动入口：`generate_context!` 每个二进制只能展开一次，故所有窗口共用此路径。
/// 成功路径在内部进入事件循环并退出进程（不返回）；构建失败返回 `Err` 供调用方兜底。
fn launch(state: AppState, view: View, popup_ipc: Option<PopupIpc>) -> tauri::Result<()> {
    let theme = window_theme(&state.config);
    let lang = Lang::resolve(&state.config.general.language);
    let window_bg = background_for(resolved_theme(&state.config));
    let popup_w = state.config.channels.popup.width;
    let popup_h = state.config.channels.popup.height;
    let always_on_top = state.config.general.always_on_top;
    let window_effect = state.config.general.window_effect;
    #[cfg(target_os = "macos")]
    let appear_behavior = state.config.general.appear_animation.ns_animation_behavior();

    // GUI Helper 模式（Daemon 拉起的弹窗进程）：弹窗是唯一渠道，恒显示窗口；作答经 IPC 回 Daemon。
    let is_helper = popup_ipc.is_some();
    // 通道启用判定（仅单进程提问模式使用）。
    let messaging_active = has_active_messaging(&state.config);
    // Helper：恒开弹窗。单进程：弹窗禁用且无可用消息渠道时兜底仍开弹窗，避免进程挂起。
    let show_popup = is_helper || state.config.channels.popup.enabled || !messaging_active;
    // 提问模式下抑制「关窗即退出」：收尾 / 等待 Daemon 收尾时弹窗会先关，需留进程主动退出。
    // 设置模式不抑制，关窗即正常退出。
    let prevent_autoexit = matches!(view, View::Popup);

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
            crate::commands::feishu_test,
            crate::commands::feishu_detect_prepare,
            crate::commands::feishu_detect_wait,
        ])
        .on_window_event(|window, event| {
            match window.label() {
                // 弹窗：关闭即取消 / 记忆尺寸。
                "popup" => match event {
                    WindowEvent::CloseRequested { .. } => {
                        let app = window.app_handle();
                        if let Some(c) = app.try_state::<Arc<Coordinator>>() {
                            // 单进程路径：投递协调器。
                            c.submit(ChannelResult::cancel("popup"));
                        } else if let Some(b) = app.try_state::<GuiBridge>() {
                            // GUI Helper 路径：经 IPC 通知 Daemon 取消。
                            b.send_cancel();
                        }
                    }
                    WindowEvent::Resized(_) => persist_popup_size(window),
                    _ => {}
                },
                // 设置窗口关闭时清掉 Liquid Glass 注册表条目：插件按 label 缓存玻璃视图，
                // 若不清理，下次同 label 重开会走 update 分支去操作已销毁的旧视图，导致背景透明无玻璃。
                #[cfg(target_os = "macos")]
                "settings" => {
                    if matches!(event, WindowEvent::CloseRequested { .. }) {
                        clear_window_glass(window);
                    }
                }
                _ => {}
            }
        })
        .setup(move |app| {
            // 裸二进制运行时 Dock 不会用 bundle 图标；运行时显式覆盖（仅影响本进程）。
            #[cfg(target_os = "macos")]
            crate::macos_dock_icon::set_dock_icon();
            match view {
                View::Popup => {
                    // Dock 跳动 + 角标提问数（仅 popup）。
                    #[cfg(target_os = "macos")]
                    {
                        let count = app.state::<AppState>().request.questions.len();
                        crate::macos_dock_icon::announce_questions(count);
                    }

                    if show_popup {
                        let builder = WebviewWindowBuilder::new(
                            app,
                            "popup",
                            WebviewUrl::App("index.html?view=popup".into()),
                        )
                        .title(i18n::tr(lang, "title.popup"))
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
                        // GUI Helper 由 Daemon 拉起，可能不自动获焦 → 尽力前置。
                        if is_helper {
                            let _ = win.set_focus();
                        }
                    }

                    match popup_ipc {
                        // —— GUI Helper 模式：作答经 IPC 回 Daemon，无本地协调器 / 消息渠道 ——
                        Some(ipc) => {
                            let PopupIpc {
                                gui_tx,
                                request_id,
                                reader,
                            } = ipc;
                            app.manage(GuiBridge {
                                tx: gui_tx,
                                request_id,
                                done: AtomicBool::new(false),
                                app: app.handle().clone(),
                            });
                            // 读 Daemon → GUI 的消息：被抢答 cancel / 连接断开 → 退出本进程。
                            let app_handle = app.handle().clone();
                            tauri::async_runtime::spawn(async move {
                                let mut reader = reader;
                                loop {
                                    match crate::ipc::read_msg::<_, crate::ipc::ServerMsg>(
                                        &mut reader,
                                    )
                                    .await
                                    {
                                        Ok(Some(crate::ipc::ServerMsg::Cancel { .. })) => {
                                            app_handle.exit(0);
                                            break;
                                        }
                                        // 配置实时变更（A12）：转发给前端实时切主题/语言。
                                        // 复用既有 "settings-updated" 事件（前端已监听 general 配置）。
                                        Ok(Some(crate::ipc::ServerMsg::ConfigChanged { general })) => {
                                            use tauri::Emitter;
                                            // 先同步原生窗口外观：玻璃/毛玻璃材质随 NSAppearance 切换，
                                            // 仅靠前端 CSS 会出现「网页变浅、窗体仍深」（见 A12 实测）。
                                            if let Some(theme) =
                                                general.get("theme").and_then(|t| t.as_str())
                                            {
                                                crate::commands::apply_theme_to_windows(
                                                    &app_handle,
                                                    theme,
                                                );
                                            }
                                            let _ = app_handle.emit("settings-updated", general);
                                        }
                                        Ok(Some(_)) => {}
                                        Ok(None) | Err(_) => {
                                            app_handle.exit(0);
                                            break;
                                        }
                                    }
                                }
                            });
                        }
                        // —— 单进程模式（非 unix 回退）：协调器 + 弹窗 Channel + 并行消息渠道 ——
                        None => {
                            let request = app.state::<AppState>().request.clone();
                            let coordinator = Coordinator::new(app.handle().clone(), request.clone());
                            if show_popup {
                                coordinator
                                    .register(Arc::new(PopupChannel::new(app.handle().clone())));
                            }
                            let config = app.state::<AppState>().config.clone();
                            for ch in active_messaging_channels(&config) {
                                coordinator.register(ch.clone());
                                ch.start(&request, coordinator.clone());
                            }
                            app.manage(coordinator);
                        }
                    }
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
    app.run(move |app_handle, event| {
        // 提问模式：拦下关窗触发的退出（code=None），由协调器 / GUI Helper 逻辑决定真正退出时机。
        // 设置模式不拦，关窗即正常退出。
        if prevent_autoexit {
            if let RunEvent::ExitRequested { code, api, .. } = &event {
                if code.is_none() {
                    if let Some(bridge) = app_handle.try_state::<GuiBridge>() {
                        // GUI Helper：关窗 / Cmd+Q → 通知 Daemon 取消，等其收尾关闭连接后由
                        // reader 驱动 `app.exit(0)`（或安全网超时），确保取消已送达 Daemon。
                        api.prevent_exit();
                        bridge.send_cancel();
                    } else {
                        // 单进程：仅在收尾阶段拦下，放行协调器 `app.exit(code)` 先输出结果。
                        let finalizing = app_handle
                            .try_state::<Arc<Coordinator>>()
                            .map(|c| c.is_finalizing())
                            .unwrap_or(false);
                        if finalizing {
                            api.prevent_exit();
                        }
                    }
                }
            }
        }
    });
    std::process::exit(0);
}

/// 渲染结果：把一个终态 `ChannelResult` 转成「给 stdout 的文本 / 给 stderr 的错误 + 退出码」。
///
/// 纯函数（除图片落盘的 IO 外），不打印、不退出，便于 Daemon 复用后经 IPC 回传 CLI。
/// 单进程路径由 `emit_result` 包一层做实际打印 / 退出。
#[derive(Debug, Clone)]
pub struct RenderOutcome {
    /// 给 CLI stdout 的结果区块文本（不含尾换行；打印方负责换行）。
    pub stdout: String,
    /// 给 CLI stderr 的错误文本（仅错误路径有值；含 `Error:` 前缀）。
    pub stderr: Option<String>,
    /// 退出码：0（发送/取消正常）/ 1（落盘等错误）。
    pub exit_code: i32,
}

/// 渲染终态结果（图片落盘到 `temp/askhuman/<request_id>/`）。文案按传入 `lang` 本地化。
pub(crate) fn render_result(request_id: &str, result: &ChannelResult, lang: Lang) -> RenderOutcome {
    match result.action {
        ChannelAction::Cancel => RenderOutcome {
            stdout: output::cancel_output(lang),
            stderr: None,
            exit_code: 0,
        },
        ChannelAction::Send => {
            // 逐题落盘图片（按题分子目录避免文件名冲突），再聚合输出。
            let mut image_paths_per_q: Vec<Vec<String>> = Vec::with_capacity(result.answers.len());
            for (i, answer) in result.answers.iter().enumerate() {
                match image_writer::save(&answer.images, request_id, i, lang) {
                    Ok(paths) => image_paths_per_q.push(paths),
                    Err(e) => {
                        return RenderOutcome {
                            stdout: String::new(),
                            stderr: Some(format!("{}{}", i18n::err_prefix(lang), e)),
                            exit_code: 1,
                        };
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

            RenderOutcome {
                stdout: output::aggregate_output(lang, &rendered),
                stderr: None,
                exit_code: 0,
            }
        }
    }
}

/// 把结果输出到 stdout（或 stderr），返回退出码。供单进程协调器调用。
pub(crate) fn emit_result(request_id: &str, result: &ChannelResult) -> i32 {
    let outcome = render_result(request_id, result, Lang::current());
    if let Some(err) = &outcome.stderr {
        stderr_redirect::eprintln_real(err);
    } else {
        println!("{}", outcome.stdout);
    }
    outcome.exit_code
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

/// 窗口关闭前移除 Liquid Glass 背景：同时把插件按 label 缓存的注册表条目清掉，
/// 以便同 label 窗口下次重建时能重新走「create」分支挂上玻璃。须在视图仍存活时调用。
#[cfg(target_os = "macos")]
fn clear_window_glass(window: &tauri::Window) {
    use tauri_plugin_liquid_glass::{LiquidGlassConfig, LiquidGlassExt};
    if let Some(w) = window.app_handle().get_webview_window(window.label()) {
        let _ = w.liquid_glass().set_effect(
            &w,
            LiquidGlassConfig {
                enabled: false,
                ..Default::default()
            },
        );
    }
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
    let lang = Lang::resolve(&config.general.language);
    let window_bg = background_for(resolved_theme(config));
    // 弹窗置顶时，设置窗口与其同级，确保新建并获焦后显示在置顶弹窗之上；
    // 无弹窗（独立 --settings 启动）或弹窗未置顶时保持普通层级，不上浮于其它 App。
    let pin_above_popup =
        manager.get_webview_window("popup").is_some() && config.general.always_on_top;
    let builder = WebviewWindowBuilder::new(
        manager,
        "settings",
        WebviewUrl::App("index.html?view=settings".into()),
    )
    .title(i18n::tr(lang, "title.settings"))
    .inner_size(560.0, 640.0)
    // 最小宽度：保证标题栏内居中的 tab 不会与左上角红绿灯重叠。
    .min_inner_size(480.0, 520.0)
    .center()
    .always_on_top(pin_above_popup)
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
fn gui_available(lang: Lang) -> Result<(), String> {
    let has_display = std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !has_display {
        return Err(i18n::tr(lang, "app.noDisplay").to_string());
    }
    if !webkitgtk_loadable() {
        return Err(i18n::tr(lang, "app.noWebkitgtk").to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn gui_available(_lang: Lang) -> Result<(), String> {
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
