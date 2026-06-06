//! 前端可调用的 Tauri 命令（弹窗模式）。

use crate::app::coordinator::Coordinator;
use crate::app::AppState;
use crate::config::{AppConfig, ThemeMode, WindowEffect};
use crate::integrations::cursor_hook;
use crate::models::{AskRequest, ChannelAction, ChannelResult, QuestionAnswer};
use crate::telegram::TelegramClient;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

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

/// 前端提交的作答内容（按问题顺序，每题一项）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupSubmission {
    #[serde(default)]
    answers: Vec<QuestionAnswer>,
}

#[tauri::command]
pub fn submit_popup(app: AppHandle, submission: PopupSubmission) {
    let result = ChannelResult {
        action: ChannelAction::Send,
        answers: submission.answers,
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
        let path = paths.get(index).ok_or_else(|| {
            crate::i18n::tr(crate::i18n::Lang::current(), "cmd.invalidAttachmentIndex").to_string()
        })?;
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
    let bytes = std::fs::read(&path).map_err(|e| {
        crate::i18n::tr(crate::i18n::Lang::current(), "cmd.readFileFailed")
            .replace("{e}", &e.to_string())
    })?;
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
        Err(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.fileIconUnsupported").to_string())
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
    cmd.spawn().map(|_| ()).map_err(|e| {
        crate::i18n::tr(crate::i18n::Lang::current(), "cmd.openFailed").replace("{e}", &e.to_string())
    })
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
pub fn save_settings(app: AppHandle, config: AppConfig) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())?;
    // 广播 general 配置，令同进程内已打开的弹窗实时生效（如语音语言/快捷键）。
    let _ = app.emit("settings-updated", &config.general);
    // 界面语言可能变化：实时更新已打开窗口的原生标题（弹窗标题在 macOS 多隐藏，settings 可见）。
    let lang = crate::i18n::Lang::resolve(&config.general.language);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.set_title(crate::i18n::tr(lang, "title.settings"));
    }
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.set_title(crate::i18n::tr(lang, "title.popup"));
    }
    Ok(())
}

#[tauri::command]
pub fn get_prompt() -> String {
    crate::prompts::cli_reference()
}

/// 设置页「弹出测试窗口」：以独立子进程跑一个示例提问，
/// 完全复用真实弹窗流程并读取已保存的配置（含出现动画），便于快速预览效果。
#[tauri::command]
pub fn open_test_popup() -> Result<(), String> {
    use std::process::{Command, Stdio};
    let lang = crate::i18n::Lang::current();
    let exe = std::env::current_exe()
        .map_err(|e| crate::i18n::tr(lang, "cmd.locateExeFailed").replace("{e}", &e.to_string()))?;
    Command::new(exe)
        .args([
            crate::i18n::tr(lang, "test.message"),
            "-q",
            crate::i18n::tr(lang, "test.question"),
            "-o",
            crate::i18n::tr(lang, "test.optionGood"),
            "-o",
            crate::i18n::tr(lang, "test.optionAdjust"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| crate::i18n::tr(lang, "cmd.testPopupFailed").replace("{e}", &e.to_string()))?;
    Ok(())
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

/// 实时切换弹窗背景效果（玻璃/模糊）到所有已打开窗口（仅 macOS 26+ 真正切换）。
/// 持久化由前端 `save_settings` 负责；此命令只负责对当前窗口即时生效。
#[tauri::command]
pub fn apply_window_effect(app: AppHandle, effect: WindowEffect) {
    for label in ["popup", "settings"] {
        if let Some(w) = app.get_webview_window(label) {
            crate::app::set_runtime_window_effect(&w, effect);
        }
    }
}

// ===== 语音输入（macOS 26 SpeechAnalyzer，离线，经 Swift 桥） =====

/// 开始语音输入：识别结果经 `speech-committed` / `speech-volatile` 等事件回传。
/// `locale` 为 BCP-47（如 zh-CN），空串=跟随系统。仅 macOS 实现；其它平台为空操作。
#[tauri::command]
pub fn start_speech(
    #[allow(unused_variables)] app: AppHandle,
    #[allow(unused_variables)] locale: Option<String>,
) {
    #[cfg(target_os = "macos")]
    crate::speech::start(app, locale.as_deref().unwrap_or(""));
}

/// 停止语音输入。仅 macOS 实现；其它平台为空操作。
#[tauri::command]
pub fn stop_speech(#[allow(unused_variables)] app: AppHandle) {
    #[cfg(target_os = "macos")]
    crate::speech::stop();
}

/// 听写中途移动光标时：固定已写文本并重启识别会话。仅 macOS。
#[tauri::command]
pub fn flush_speech(#[allow(unused_variables)] app: AppHandle) {
    #[cfg(target_os = "macos")]
    crate::speech::flush();
}

/// 语音输入是否可用（macOS 26+）。非 macOS 或低版本返回 false。
#[tauri::command]
pub fn speech_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        return crate::speech::is_available();
    }
    #[allow(unreachable_code)]
    false
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
    let lang = crate::i18n::Lang::current();
    let client = TelegramClient::new(args.bot_token, args.chat_id, args.api_base_url)
        .map_err(|e| e.localized(lang))?;
    client.test_connection(lang).await.map_err(|e| e.localized(lang))
}

// ===== 钉钉测试连接 / userId 自动识别 =====

use crate::config::DingTalkChannelConfig;
use crate::dingtalk::client::DingTalkClient;
use crate::dingtalk::stream::{StreamConn, StreamEvent, TOPIC_BOT_MESSAGE};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkTestArgs {
    client_id: String,
    client_secret: String,
    user_id: String,
}

/// 测试连接：换 token（校验 ClientId/Secret）+ 向 userId 单聊发一条测试消息。
#[tauri::command]
pub async fn dingtalk_test(args: DingTalkTestArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    if args.user_id.trim().is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillUserId").to_string());
    }
    let cfg = DingTalkChannelConfig {
        enabled: true,
        client_id: args.client_id,
        client_secret: args.client_secret,
        user_id: args.user_id,
        card_template_id: String::new(),
        ..Default::default()
    };
    let client = DingTalkClient::new(&cfg).map_err(|e| e.localized(lang))?;
    client
        .send_oto_text(crate::i18n::tr(lang, "cmd.ddTestRemote"))
        .await
        .map_err(|e| e.localized(lang))?;
    Ok(crate::i18n::tr(lang, "cmd.ddTestSent").to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkDetectArgs {
    client_id: String,
    client_secret: String,
}

/// 自动识别准备：校验 ClientId/Secret（换 token），通过后返回供用户私聊发送的 4 位识别码。
/// 校验不通过则返回中文错误（前端据此不展示识别码、不进入等待）。
#[tauri::command]
pub async fn dingtalk_detect_prepare(args: DingTalkDetectArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    let client_id = args.client_id.trim();
    let client_secret = args.client_secret.trim();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillClientIdSecret").to_string());
    }
    let http = reqwest::Client::new();
    crate::dingtalk::token::get_token(&http, client_id, client_secret)
        .await
        .map_err(|e| e.localized(lang))?;
    Ok(gen_detect_code())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkWaitArgs {
    client_id: String,
    client_secret: String,
    code: String,
}

/// 自动识别等待：开 Stream（bot 消息 topic），等到内容等于识别码的单聊消息，返回其 senderStaffId。
/// 120 秒超时报错。
#[tauri::command]
pub async fn dingtalk_detect_wait(args: DingTalkWaitArgs) -> Result<String, String> {
    use std::time::Duration;
    let lang = crate::i18n::Lang::current();
    let client_id = args.client_id.trim();
    let client_secret = args.client_secret.trim();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillClientIdSecret").to_string());
    }
    let code = args.code.trim().to_string();
    if code.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
    }
    let http = reqwest::Client::new();
    let mut stream = StreamConn::connect(http, client_id, client_secret, &[TOPIC_BOT_MESSAGE])
        .await
        .map_err(|e| e.localized(lang))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string());
        }
        match tokio::time::timeout(remaining, stream.recv()).await {
            Ok(Some(StreamEvent::BotMessage(data))) => {
                let content = data
                    .get("text")
                    .and_then(|t| t.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .trim();
                if content == code {
                    if let Some(sender) =
                        data.get("senderStaffId").and_then(|v| v.as_str())
                    {
                        return Ok(sender.to_string());
                    }
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => return Err(crate::i18n::tr(lang, "cmd.streamDisconnected").to_string()),
            Err(_) => return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string()),
        }
    }
}

// ===== 飞书测试连接 / open_id 自动识别 =====

use crate::config::FeishuChannelConfig;
use crate::feishu::client::FeishuClient;
use crate::feishu::ws::{FeishuWs, WsEvent};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuTestArgs {
    app_id: String,
    app_secret: String,
    open_id: String,
    base_url: String,
}

/// 测试连接：换 token（校验 AppId/Secret）+ 向 open_id 单聊发一条测试消息。
#[tauri::command]
pub async fn feishu_test(args: FeishuTestArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    if args.open_id.trim().is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillOpenId").to_string());
    }
    let cfg = FeishuChannelConfig {
        enabled: true,
        app_id: args.app_id,
        app_secret: args.app_secret,
        open_id: args.open_id,
        base_url: args.base_url,
    };
    let client = FeishuClient::new(&cfg).map_err(|e| e.localized(lang))?;
    client
        .send_text(crate::i18n::tr(lang, "cmd.fsTestRemote"))
        .await
        .map_err(|e| e.localized(lang))?;
    Ok(crate::i18n::tr(lang, "cmd.fsTestSent").to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuDetectArgs {
    app_id: String,
    app_secret: String,
    base_url: String,
}

/// 自动识别准备：校验 AppId/Secret（换 token），通过后返回供用户私聊发送的 4 位识别码。
#[tauri::command]
pub async fn feishu_detect_prepare(args: FeishuDetectArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    let app_id = args.app_id.trim();
    let app_secret = args.app_secret.trim();
    if app_id.is_empty() || app_secret.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillAppIdSecret").to_string());
    }
    let base_url = effective_feishu_base(&args.base_url);
    let http = reqwest::Client::new();
    crate::feishu::token::get_token(&http, &base_url, app_id, app_secret)
        .await
        .map_err(|e| e.localized(lang))?;
    Ok(gen_detect_code())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuWaitArgs {
    app_id: String,
    app_secret: String,
    base_url: String,
    code: String,
}

/// 自动识别等待：开长连接，等到内容等于识别码的单聊消息，返回发送者 open_id。120 秒超时报错。
#[tauri::command]
pub async fn feishu_detect_wait(args: FeishuWaitArgs) -> Result<String, String> {
    use std::time::Duration;
    let lang = crate::i18n::Lang::current();
    let app_id = args.app_id.trim();
    let app_secret = args.app_secret.trim();
    if app_id.is_empty() || app_secret.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillAppIdSecret").to_string());
    }
    let code = args.code.trim().to_string();
    if code.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
    }
    let base_url = effective_feishu_base(&args.base_url);
    let http = reqwest::Client::new();
    let mut ws = FeishuWs::connect(http, &base_url, app_id, app_secret)
        .await
        .map_err(|e| e.localized(lang))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string());
        }
        match tokio::time::timeout(remaining, ws.recv()).await {
            Ok(Some(WsEvent::Message(event))) => {
                if let Some((open_id, text)) = feishu_text_and_sender(&event) {
                    if text.trim() == code {
                        return Ok(open_id);
                    }
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => return Err(crate::i18n::tr(lang, "cmd.streamDisconnected").to_string()),
            Err(_) => return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string()),
        }
    }
}

/// base_url 缺省回退飞书国内。
fn effective_feishu_base(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    if b.is_empty() {
        "https://open.feishu.cn".to_string()
    } else {
        b.to_string()
    }
}

/// 从 im.message.receive_v1 的 event 取 (发送者 open_id, 文本内容)。非文本消息返回 None。
fn feishu_text_and_sender(event: &serde_json::Value) -> Option<(String, String)> {
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
    let content_str = message.get("content").and_then(|v| v.as_str()).unwrap_or("{}");
    let content: serde_json::Value = serde_json::from_str(content_str).ok()?;
    let text = content.get("text").and_then(|v| v.as_str())?.to_string();
    Some((open_id, text))
}

/// 生成 4 位识别码（瞬时配对用，无需强随机）。
fn gen_detect_code() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{:04}", nanos % 10000)
}
