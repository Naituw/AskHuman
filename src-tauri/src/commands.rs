//! 前端可调用的 Tauri 命令（弹窗模式）。

use crate::app::coordinator::Coordinator;
use crate::app::AppState;
use crate::config::{AppConfig, ThemeMode};
use crate::integrations::cursor_hook;
use crate::models::{AskRequest, ChannelAction, ChannelResult, ImageAttachment};
use crate::telegram::TelegramClient;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// QuickLook 预览会话的共享状态：用「代次」区分「切换」与「真正关闭」。
#[derive(Default, Clone)]
pub struct PreviewShared {
    generation: Arc<AtomicU64>,
    pid: Arc<Mutex<Option<u32>>>,
}

/// 弹窗初始化负载：请求内容 + 主题 + 是否置顶（前端据此套用样式、初始化导航栏）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupInit {
    request: AskRequest,
    theme: String,
    always_on_top: bool,
}

#[tauri::command]
pub fn popup_init(state: State<AppState>) -> PopupInit {
    PopupInit {
        request: state.request.clone(),
        theme: theme_str(state.config.general.theme),
        always_on_top: state.config.general.always_on_top,
    }
}

/// 前端提交的作答内容。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupSubmission {
    #[serde(default)]
    selected_options: Vec<String>,
    #[serde(default)]
    user_input: String,
    #[serde(default)]
    images: Vec<ImageAttachment>,
}

#[tauri::command]
pub fn submit_popup(app: AppHandle, submission: PopupSubmission) {
    let result = ChannelResult {
        action: ChannelAction::Send,
        selected_options: submission.selected_options,
        user_input: Some(submission.user_input),
        images: submission.images,
        source_channel_id: "popup".to_string(),
    };
    if let Some(c) = app.try_state::<Arc<Coordinator>>() {
        c.submit(result);
    }
}

#[tauri::command]
pub fn cancel_popup(app: AppHandle) {
    if let Some(c) = app.try_state::<Arc<Coordinator>>() {
        c.submit(ChannelResult::cancel("popup"));
    }
}

// ===== 文件附件：打开 / 预览 / 缩略图 =====

/// 用系统默认程序打开文件（macOS open / Windows start / Linux xdg-open）。
#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    open_with_system(&path)
}

/// 预览文件：macOS 走 QuickLook（qlmanage -p），其它平台回退为「打开」。
/// `restore_pin`：弹窗原本是否置顶。预览期间临时取消置顶（否则 QuickLook 被压在下面），
/// QuickLook 真正关闭后恢复为该值。
///
/// 切换：若已有预览在开，先结束旧预览再开新的（前端「预览中单击其它附件」即调用此处）。
/// 用 generation 区分「被切换替换」与「用户关闭」：仅后者恢复置顶并发 `preview-closed`。
#[tauri::command]
pub fn preview_path(
    app: AppHandle,
    #[allow(unused_variables)] state: State<PreviewShared>,
    path: String,
    #[allow(unused_variables)] restore_pin: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};
        let shared = state.inner().clone();
        let my_gen = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;

        // 结束上一个预览（切换）。
        if let Some(old_pid) = shared.pid.lock().unwrap().take() {
            unsafe {
                libc::kill(old_pid as i32, libc::SIGTERM);
            }
        }

        if let Some(w) = app.get_webview_window("popup") {
            let _ = w.set_always_on_top(false);
        }

        let mut child = Command::new("qlmanage")
            .arg("-p")
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("预览失败: {}", e))?;
        *shared.pid.lock().unwrap() = Some(child.id());
        let _ = app.emit("preview-opened", ());

        let app2 = app.clone();
        let shared2 = shared.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            // 仍是当前代次 → 用户真正关闭了预览（非被切换替换）。
            if shared2.generation.load(Ordering::SeqCst) == my_gen {
                *shared2.pid.lock().unwrap() = None;
                if let Some(w) = app2.get_webview_window("popup") {
                    let _ = w.set_always_on_top(restore_pin);
                }
                let _ = app2.emit("preview-closed", ());
            }
        });
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        open_with_system(&path)
    }
}

/// 读取本地图片并返回 base64 data URL（供前端缩略图显示）。
#[tauri::command]
pub fn read_image_data_url(path: String) -> Result<String, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败: {}", e))?;
    let mime = image_mime_from_path(&path);
    Ok(format!("data:{};base64,{}", mime, B64.encode(bytes)))
}

fn open_with_system(path: &str) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("打开失败: {}", e))
}

fn image_mime_from_path(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn theme_str(theme: ThemeMode) -> String {
    match theme {
        ThemeMode::System => "system",
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
    }
    .to_string()
}

// ===== 设置页命令 =====

#[tauri::command]
pub fn get_settings() -> AppConfig {
    AppConfig::load()
}

#[tauri::command]
pub fn save_settings(config: AppConfig) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_prompt() -> &'static str {
    crate::prompts::CLI_REFERENCE
}

/// 实时应用主题到已打开的窗口（system→跟随系统）。
#[tauri::command]
pub fn set_theme(app: AppHandle, theme: String) {
    apply_theme_to_windows(&app, &theme);
}

/// 从弹窗导航栏切换主题：写入配置并实时应用到所有窗口。
#[tauri::command]
pub fn update_theme(app: AppHandle, theme: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.general.theme = match theme.as_str() {
        "light" => ThemeMode::Light,
        "dark" => ThemeMode::Dark,
        _ => ThemeMode::System,
    };
    cfg.save().map_err(|e| e.to_string())?;
    apply_theme_to_windows(&app, &theme);
    Ok(())
}

fn apply_theme_to_windows(app: &AppHandle, theme: &str) {
    let t = match theme {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    for label in ["settings", "popup"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.set_theme(t);
        }
    }
}

/// 从弹窗导航栏打开设置窗口（同进程内创建，不影响弹窗等待）。
#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    crate::app::create_settings_window(&app, &AppConfig::load()).map_err(|e| e.to_string())
}

// ===== Cursor Hook =====

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    installed: bool,
    hooks_json_exists: bool,
    supported: bool,
}

#[tauri::command]
pub fn cursor_hook_status() -> HookStatus {
    HookStatus {
        installed: cursor_hook::is_installed(),
        hooks_json_exists: cursor_hook::hooks_json_exists(),
        supported: cursor_hook::supported(),
    }
}

#[tauri::command]
pub fn cursor_hook_install() -> Result<String, String> {
    cursor_hook::install().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cursor_hook_uninstall() -> Result<String, String> {
    cursor_hook::uninstall().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cursor_hook_reveal() {
    cursor_hook::reveal();
}

// ===== Telegram 测试连接 =====

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramTestArgs {
    bot_token: String,
    chat_id: String,
    api_base_url: String,
}

#[tauri::command]
pub async fn telegram_test(args: TelegramTestArgs) -> Result<String, String> {
    let client = TelegramClient::new(args.bot_token, args.chat_id, args.api_base_url)
        .map_err(|e| e.to_string())?;
    client.test_connection().await.map_err(|e| e.to_string())
}
