//! Tauri 运行时：创建窗口、运行事件循环、汇集 Channel 结果并退出。

use crate::cli::{image_writer, output};
use crate::config::{AppConfig, ThemeMode};
use crate::models::{AskRequest, ChannelAction, ChannelResult};
use std::io::Write;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// 运行时共享状态。
pub struct AppState {
    pub request: AskRequest,
    pub config: AppConfig,
    /// 抢答门闩：首个终态结果生效，其余忽略。
    pub finished: Mutex<bool>,
}

#[derive(Clone, Copy)]
enum View {
    Popup,
    Settings,
}

/// 提问模式：创建弹窗，等待用户作答 / 取消，输出结果并退出进程。
pub fn run_ask(request: AskRequest, config: AppConfig) -> ! {
    launch(
        AppState {
            request,
            config,
            finished: Mutex::new(false),
        },
        View::Popup,
    )
}

/// 设置模式：创建设置窗口（前端界面在 Step 5 完善）。
pub fn run_settings(config: AppConfig) -> ! {
    launch(
        AppState {
            request: AskRequest::new(String::new(), Vec::new(), false),
            config,
            finished: Mutex::new(false),
        },
        View::Settings,
    )
}

/// 统一启动入口：`generate_context!` 每个二进制只能展开一次，故所有窗口共用此路径。
fn launch(state: AppState, view: View) -> ! {
    let theme = window_theme(&state.config);
    let popup_w = state.config.channels.popup.width;
    let popup_h = state.config.channels.popup.height;
    let always_on_top = state.config.general.always_on_top;

    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            crate::commands::popup_init,
            crate::commands::submit_popup,
            crate::commands::cancel_popup,
        ])
        .on_window_event(|window, event| {
            // 仅弹窗参与“关闭即取消 / 记忆尺寸”；设置窗口走默认关闭行为。
            if window.label() != "popup" {
                return;
            }
            match event {
                WindowEvent::CloseRequested { .. } => {
                    finish(window.app_handle(), ChannelResult::cancel("popup"));
                }
                WindowEvent::Resized(_) => persist_popup_size(window),
                _ => {}
            }
        })
        .setup(move |app| {
            match view {
                View::Popup => {
                    // 先隐藏创建，给原生窗口涂上最终主题背景色后立即显示：
                    // 窗口瞬间出现且已是目标底色，既无白屏闪烁也无可感知延迟。
                    let window = WebviewWindowBuilder::new(
                        app,
                        "popup",
                        WebviewUrl::App("index.html?view=popup".into()),
                    )
                    .title("HumanInLoop")
                    .inner_size(popup_w, popup_h)
                    .min_inner_size(420.0, 480.0)
                    .center()
                    .always_on_top(always_on_top)
                    .visible(false)
                    .theme(theme)
                    .build()?;

                    // theme() 返回解析后的实际主题（system 亦可），据此取底色。
                    let resolved = window.theme().unwrap_or(tauri::Theme::Light);
                    let _ = window.set_background_color(Some(background_for(resolved)));
                    let _ = window.show();
                    let _ = window.set_focus();
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
                    .theme(theme)
                    .build()?;
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("启动 Tauri 失败");

    // 构建成功后、进入事件循环前静默系统噪音日志（如 macOS 的 TSM CapsLock 日志）。
    // 此前的参数/构建错误已照常打到真正的 stderr。
    stderr_redirect::silence();
    app.run(|_app_handle, _event| {});
    std::process::exit(0);
}

/// 收口：首个结果生效，输出到 stdout 后退出进程。
pub(crate) fn finish(app: &AppHandle, result: ChannelResult) {
    let state = app.state::<AppState>();
    {
        let mut done = state.finished.lock().unwrap();
        if *done {
            return;
        }
        *done = true;
    }
    let code = emit_result(&state.request, &result);
    let _ = std::io::stdout().flush();
    app.exit(code);
}

fn emit_result(request: &AskRequest, result: &ChannelResult) -> i32 {
    match result.action {
        ChannelAction::Cancel => {
            println!("{}", output::cancel_output());
            0
        }
        ChannelAction::Send => match image_writer::save(&result.images, &request.id) {
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
                // stderr 已被静默，写回保存的真实 stderr。
                stderr_redirect::eprintln_real(&format!("错误: {}", e));
                1
            }
        },
    }
}

/// 原生窗口底色（与前端 tokens.css `--bg` 对齐）。
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
            let devnull = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_WRONLY);
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
