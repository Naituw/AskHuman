//! Tauri 运行时：创建窗口、并行启动 Channel、汇集结果并退出。

pub mod coordinator;

use crate::channels::popup::PopupChannel;
use crate::channels::telegram::TelegramChannel;
use crate::channels::Channel;
use crate::cli::{image_writer, output};
use crate::config::{AppConfig, ThemeMode};
use crate::models::{AskRequest, ChannelAction, ChannelResult};
use crate::telegram::TelegramClient;
use coordinator::Coordinator;
use std::sync::Arc;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

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

/// 提问模式：创建弹窗 + 并行 Channel，等待首个结果，输出并退出。
pub fn run_ask(request: AskRequest, config: AppConfig) -> ! {
    launch(AppState { request, config }, View::Popup)
}

/// 设置模式：创建设置窗口。
pub fn run_settings(config: AppConfig) -> ! {
    launch(
        AppState {
            request: AskRequest::new(String::new(), Vec::new(), false),
            config,
        },
        View::Settings,
    )
}

/// 统一启动入口：`generate_context!` 每个二进制只能展开一次，故所有窗口共用此路径。
fn launch(state: AppState, view: View) -> ! {
    let theme = window_theme(&state.config);
    let window_bg = background_for(resolved_theme(&state.config));
    let popup_w = state.config.channels.popup.width;
    let popup_h = state.config.channels.popup.height;
    let always_on_top = state.config.general.always_on_top;

    // 通道启用判定（仅提问模式使用）。
    let tg = state.config.channels.telegram.clone();
    let telegram_active = tg.enabled
        && TelegramClient::new(tg.bot_token.clone(), tg.chat_id.clone(), tg.api_base_url.clone())
            .is_ok();
    // 弹窗禁用且 Telegram 不可用时，兜底仍开弹窗，避免无任何 Channel 导致进程挂起。
    let show_popup = state.config.channels.popup.enabled || !telegram_active;

    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            crate::commands::popup_init,
            crate::commands::submit_popup,
            crate::commands::cancel_popup,
            crate::commands::get_settings,
            crate::commands::save_settings,
            crate::commands::get_prompt,
            crate::commands::set_theme,
            crate::commands::cursor_hook_status,
            crate::commands::cursor_hook_install,
            crate::commands::cursor_hook_uninstall,
            crate::commands::cursor_hook_reveal,
            crate::commands::telegram_test,
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
            match view {
                View::Popup => {
                    let request = app.state::<AppState>().request.clone();
                    let coordinator = Coordinator::new(app.handle().clone(), request.clone());

                    if show_popup {
                        WebviewWindowBuilder::new(
                            app,
                            "popup",
                            WebviewUrl::App("index.html?view=popup".into()),
                        )
                        .title("HumanInLoop")
                        .inner_size(popup_w, popup_h)
                        .min_inner_size(420.0, 480.0)
                        .center()
                        .always_on_top(always_on_top)
                        .background_color(window_bg)
                        .theme(theme)
                        .build()?;
                        coordinator.register(Arc::new(PopupChannel::new(app.handle().clone())));
                    }

                    if telegram_active {
                        let ch = Arc::new(TelegramChannel::new(tg.clone()));
                        coordinator.register(ch.clone());
                        ch.start(&request, coordinator.clone());
                    }

                    app.manage(coordinator);
                }
                View::Settings => {
                    WebviewWindowBuilder::new(
                        app,
                        "settings",
                        WebviewUrl::App("index.html?view=settings".into()),
                    )
                    .title("HumanInLoop 设置")
                    .inner_size(560.0, 640.0)
                    .center()
                    .background_color(window_bg)
                    .theme(theme)
                    .build()?;
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("启动 Tauri 失败");

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
        ChannelAction::Send => match image_writer::save(&result.images, request_id) {
            Ok(paths) => {
                println!(
                    "{}",
                    output::send_output(
                        &result.selected_options,
                        result.user_input.as_deref(),
                        &paths,
                    )
                );
                0
            }
            Err(e) => {
                stderr_redirect::eprintln_real(&format!("错误: {}", e));
                1
            }
        },
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
