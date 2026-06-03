//! 前端可调用的 Tauri 命令（弹窗模式）。

use crate::app::coordinator::Coordinator;
use crate::app::AppState;
use crate::config::{AppConfig, ThemeMode};
use crate::integrations::cursor_hook;
use crate::models::{AskRequest, ChannelAction, ChannelResult, ImageAttachment};
use crate::telegram::TelegramClient;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

/// 弹窗初始化负载：请求内容 + 主题 + 是否置顶（前端据此套用样式、初始化导航栏）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupInit {
    request: AskRequest,
    theme: String,
    always_on_top: bool,
    /// 标题来源名：「Question from {source_name}」。可经环境变量定制。
    source_name: String,
}

#[tauri::command]
pub fn popup_init(state: State<AppState>) -> PopupInit {
    PopupInit {
        request: state.request.clone(),
        theme: theme_str(state.config.general.theme),
        always_on_top: state.config.general.always_on_top,
        source_name: crate::models::source_name(),
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
    /// 用户拖入回复的非图片文件绝对路径（直接透传）。
    #[serde(default)]
    files: Vec<String>,
}

#[tauri::command]
pub fn submit_popup(app: AppHandle, submission: PopupSubmission) {
    let result = ChannelResult {
        action: ChannelAction::Send,
        selected_options: submission.selected_options,
        user_input: Some(submission.user_input),
        images: submission.images,
        files: submission.files,
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

/// 预览附件：macOS 用原生 QLPreviewPanel 展示「全部附件」并定位到 `index`，
/// 面板内方向键即可在附件间切换（与 Finder 一致）；其它平台回退为「打开」当前项。
#[tauri::command]
pub fn preview_attachments(
    app: AppHandle,
    paths: Vec<String>,
    index: usize,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // 取弹窗 NSWindow 指针：把预览控制者插入其响应链，方可经协议控制面板。
        let win_ptr = app
            .get_webview_window("popup")
            .and_then(|w| w.ns_window().ok())
            .map(|p| p as usize)
            .unwrap_or(0);
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            crate::macos_quicklook::show(app2, win_ptr, &paths, index);
        });
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        let path = paths.get(index).ok_or_else(|| "无效的附件索引".to_string())?;
        open_with_system(path)
    }
}

/// 关闭当前 QuickLook 预览（点击附件以外区域时调用）。
#[tauri::command]
pub fn close_preview(#[allow(unused_variables)] app: AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(|| {
            crate::macos_quicklook::hide();
        });
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

/// 获取文件的系统图标（macOS：NSWorkspace，Finder 同款）并返回 PNG data URL，
/// 供前端把 -f 附件胶囊拖出到其它应用时作为拖拽预览图标。
#[tauri::command]
pub fn file_icon_data_url(app: AppHandle, path: String) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        use std::sync::mpsc::channel;
        let (tx, rx) = channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(crate::macos_quicklook::file_icon_png_base64(&path));
        })
        .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, path);
        Err("当前平台不支持获取文件图标".into())
    }
}

/// 弹出 -f 附件胶囊的原生右键菜单（Finder 风格）。macOS 专属，其它平台为空操作。
#[tauri::command]
pub fn show_attachment_menu(app: AppHandle, path: String) {
    #[cfg(target_os = "macos")]
    {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            crate::macos_menu::show(app2, path);
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, path);
    }
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
