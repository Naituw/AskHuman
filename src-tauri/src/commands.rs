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
        // GUI Helper 模式下来源名由 Daemon 上送（A11）；单进程 / 设置回退取本进程环境。
        source_name: state.source.clone(),
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
    // GUI Helper 模式：经 IPC 回传 Daemon。
    if let Some(bridge) = app.try_state::<crate::app::GuiBridge>() {
        bridge.send_answer(submission.answers);
        return;
    }
    // 单进程（非 unix 回退）模式：投递本地协调器。
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
    if let Some(bridge) = app.try_state::<crate::app::GuiBridge>() {
        bridge.send_cancel();
        return;
    }
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
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    index: usize,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // 取【调用方窗口】NSWindow 指针（弹窗或历史窗口）：把预览控制者插入其响应链，
        // 方可经协议控制面板（方向键切换）。回退到 popup 以兼容历史调用方。
        let win_ptr = window
            .ns_window()
            .ok()
            .or_else(|| {
                app.get_webview_window("popup")
                    .and_then(|w| w.ns_window().ok())
            })
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
        let _ = (app, window);
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
        crate::i18n::tr(crate::i18n::Lang::current(), "cmd.openFailed")
            .replace("{e}", &e.to_string())
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

// ===== 回复历史 =====

/// 历史窗口初始化负载：当前主题 + 当前项目（用于默认过滤）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryInit {
    theme: String,
    /// 当前项目 key（可空）。
    project: String,
    /// 当前项目显示名（basename；可空）。
    project_name: String,
}

#[tauri::command]
pub fn history_init(state: State<AppState>) -> HistoryInit {
    HistoryInit {
        theme: theme_str(state.config.general.theme),
        project: state.project.clone(),
        project_name: crate::project::display_name(&state.project),
    }
}

/// Agent 状态窗口初始化负载（实验性功能 spec D13）：主题 + 语言（前端据此渲染样式与文案）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsInit {
    theme: String,
    lang: String,
}

#[tauri::command]
pub fn agents_init(state: State<AppState>) -> AgentsInit {
    AgentsInit {
        theme: theme_str(state.config.general.theme),
        lang: crate::i18n::Lang::resolve(&state.config.general.language)
            .code()
            .to_string(),
    }
}

/// 由 Agent 状态窗口前端在 `agents-updated` 监听就绪后调用，启动到 daemon 的快照订阅
/// （幂等）。延迟到此刻才连 daemon，是为避免 daemon 的首帧立即快照早于前端监听而丢失。
#[tauri::command]
pub fn agents_start_subscription(app: AppHandle) {
    #[cfg(unix)]
    crate::app::start_agents_subscription(app);
    #[cfg(not(unix))]
    let _ = app;
}

/// 把「打开窗口」请求路由到统一 GUI 宿主（spec D3）：宿主在则聚焦/新建（全局单窗），不在则拉起。
/// 失败兜底：在当前（弹窗）进程内直接建窗，保证按钮始终能开窗。整个过程在后台线程进行，
/// 避免阻塞调用方（弹窗 UI 线程）——`host_open` 在宿主冷启动时可能耗时上百毫秒到数秒。
#[cfg(unix)]
fn route_open_window(
    app: AppHandle,
    kind: crate::gui_host::WindowKind,
    all: bool,
    project: Option<String>,
) {
    use crate::gui_host::WindowKind;
    std::thread::spawn(move || {
        if crate::gui_host::host_open(kind, all, project.clone()).is_ok() {
            return;
        }
        let fallback = app.clone();
        let _ = app.run_on_main_thread(move || {
            let cfg = AppConfig::load_without_secrets();
            // 兜底在弹窗进程内建窗：沿用进程内置顶判定（有弹窗且置顶 → 浮于其上）。
            let pin = crate::app::popup_pin(&fallback, &cfg);
            let _ = match kind {
                WindowKind::Settings => crate::app::create_settings_window(&fallback, &cfg, pin),
                WindowKind::History => {
                    crate::app::create_history_window(&fallback, &cfg, all, project.as_deref(), pin)
                }
                WindowKind::Agents => crate::app::create_agents_window(&fallback, &cfg),
            };
        });
    });
}

/// 从弹窗导航栏打开独立历史窗口：路由到统一宿主（全局单窗），默认过滤到弹窗所属项目。
#[tauri::command]
pub fn open_history(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    #[cfg(unix)]
    {
        route_open_window(
            app,
            crate::gui_host::WindowKind::History,
            false,
            Some(state.project.clone()),
        );
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = &state;
        let cfg = AppConfig::load_without_secrets();
        let pin = crate::app::popup_pin(&app, &cfg);
        crate::app::create_history_window(&app, &cfg, false, None, pin).map_err(|e| e.to_string())
    }
}

/// 读取历史记录：`all` 为 true 时返回全部项目，否则按 `project`（缺省空串）过滤；按时间倒序。
#[tauri::command]
pub fn get_history(project: Option<String>, all: bool) -> Vec<crate::history::HistoryEntry> {
    crate::history::load(project.as_deref(), all)
}

/// 历史中出现过的项目列表（供窗口下拉切换）。
#[tauri::command]
pub fn get_history_projects() -> Vec<crate::history::ProjectInfo> {
    crate::history::projects()
}

/// 当前历史总条数（设置页据此判断是否超额）。
#[tauri::command]
pub fn history_count() -> usize {
    crate::history::count()
}

/// 立即把历史裁剪到 `limit` 条（设置页「立即清理」）。返回裁剪后条数。
#[tauri::command]
pub fn trim_history(limit: u32) -> usize {
    crate::history::trim(limit)
}

/// 清空历史：`all` 为 true 清全部，否则清 `project`（缺省空串）。
#[tauri::command]
pub fn clear_history(all: bool, project: Option<String>) {
    let scope = if all {
        crate::history::ClearScope::All
    } else {
        crate::history::ClearScope::Project(project.unwrap_or_default())
    };
    crate::history::clear(scope);
}

// ===== 设置页命令 =====

/// Whether each channel secret is currently configured (keychain or plaintext fallback). Drives
/// the settings page "Saved" placeholder without ever exposing the secret value.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsPresent {
    dingding_secret: bool,
    feishu_secret: bool,
    telegram_token: bool,
    slack_bot_token: bool,
    slack_app_token: bool,
}

/// Settings payload: the config with secrets blanked, plus per-secret presence flags.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPayload {
    config: AppConfig,
    secrets_present: SecretsPresent,
}

/// Per-secret edit intent sent by the settings page on save. The secret value never round-trips
/// through the config object; it is carried only here (and only for `set`).
#[derive(Deserialize, Default)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SecretAction {
    /// Keep the stored secret as-is (the user did not touch the field).
    #[default]
    Unchanged,
    /// Replace the stored secret with `value`.
    Set { value: String },
    /// Delete the stored secret from the keychain.
    Clear,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SecretActions {
    dingding_secret: SecretAction,
    feishu_secret: SecretAction,
    telegram_token: SecretAction,
    slack_bot_token: SecretAction,
    slack_app_token: SecretAction,
}

#[tauri::command]
pub fn get_settings() -> SettingsPayload {
    let mut config = AppConfig::load();
    // Presence is derived from the resolved value (keychain first, plaintext fallback).
    let secrets_present = SecretsPresent {
        dingding_secret: !config.channels.dingding.client_secret.is_empty(),
        feishu_secret: !config.channels.feishu.app_secret.is_empty(),
        telegram_token: !config.channels.telegram.bot_token.is_empty(),
        slack_bot_token: !config.channels.slack.bot_token.is_empty(),
        slack_app_token: !config.channels.slack.app_token.is_empty(),
    };
    // Never let resolved secrets reach the UI; the page shows a fixed-length placeholder instead.
    config.channels.dingding.client_secret.clear();
    config.channels.feishu.app_secret.clear();
    config.channels.telegram.bot_token.clear();
    config.channels.slack.bot_token.clear();
    config.channels.slack.app_token.clear();
    SettingsPayload {
        config,
        secrets_present,
    }
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    mut config: AppConfig,
    secret_actions: SecretActions,
) -> Result<(), String> {
    // Secrets are governed solely by the explicit actions (the incoming config carries blank
    // placeholders). unchanged → leave the field empty so save() won't touch the keychain;
    // set → store it via save(); clear → delete from the keychain now.
    apply_secret_action(
        &mut config.channels.dingding.client_secret,
        crate::secrets::ACCOUNT_DINGTALK_SECRET,
        secret_actions.dingding_secret,
    );
    apply_secret_action(
        &mut config.channels.feishu.app_secret,
        crate::secrets::ACCOUNT_FEISHU_SECRET,
        secret_actions.feishu_secret,
    );
    apply_secret_action(
        &mut config.channels.telegram.bot_token,
        crate::secrets::ACCOUNT_TELEGRAM_TOKEN,
        secret_actions.telegram_token,
    );
    apply_secret_action(
        &mut config.channels.slack.bot_token,
        crate::secrets::ACCOUNT_SLACK_BOT_TOKEN,
        secret_actions.slack_bot_token,
    );
    apply_secret_action(
        &mut config.channels.slack.app_token,
        crate::secrets::ACCOUNT_SLACK_APP_TOKEN,
        secret_actions.slack_app_token,
    );
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

/// Apply one secret's edit intent to the in-memory config field before persisting.
fn apply_secret_action(field: &mut String, account: &str, action: SecretAction) {
    match action {
        SecretAction::Unchanged => field.clear(),
        SecretAction::Set { value } => *field = value,
        SecretAction::Clear => {
            let _ = crate::secrets::delete(account);
            field.clear();
        }
    }
}

/// Resolve the secret to use for a test/detect call. The settings form sends an empty secret when
/// the user kept the "Saved" placeholder; fall back to the effective secret (keychain or plaintext
/// config fallback) so they need not retype it. A non-empty `provided` always wins.
fn fallback_secret(provided: &str, pick: impl FnOnce(&AppConfig) -> String) -> String {
    if !provided.trim().is_empty() {
        return provided.to_string();
    }
    pick(&AppConfig::load())
}

/// 返回参考提示词正文：`variant = "mcp"` → MCP 版；其余（含缺省）→ CLI 版。
/// 供手动集成卡按 CLI/MCP 切换展示。
#[tauri::command]
pub fn get_prompt(variant: Option<String>) -> String {
    match variant.as_deref() {
        Some("mcp") => crate::prompts::mcp_reference(),
        _ => crate::prompts::cli_reference(),
    }
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

/// Popup sound platform support for settings UI rendering.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupSoundSupport {
    /// `"named"` (macOS with `names`), `"toggle"` (Linux), or `"none"` (hidden).
    kind: String,
    /// Optional sound names, only non-empty for `"named"`.
    names: Vec<String>,
}

#[tauri::command]
pub fn popup_sound_support() -> PopupSoundSupport {
    PopupSoundSupport {
        kind: crate::sound::support().to_string(),
        names: crate::sound::names(),
    }
}

/// Settings preview action. Empty string does not play anything.
#[tauri::command]
pub fn play_popup_sound(name: String) {
    crate::sound::play(&name);
}

/// 实时应用主题到已打开的窗口（system→跟随系统）。
#[tauri::command]
pub fn set_theme(app: AppHandle, theme: String) {
    apply_theme_to_windows(&app, &theme);
}

/// 从弹窗导航栏切换主题：写入配置并实时应用到所有窗口。
#[tauri::command]
pub fn update_theme(app: AppHandle, theme: String) -> Result<(), String> {
    // Only the theme changes; load without resolving secrets so save() neither reads nor rewrites
    // the keychain (blank secret fields are left as-is by save()).
    let mut cfg = AppConfig::load_without_secrets();
    cfg.general.theme = match theme.as_str() {
        "light" => ThemeMode::Light,
        "dark" => ThemeMode::Dark,
        _ => ThemeMode::System,
    };
    cfg.save().map_err(|e| e.to_string())?;
    apply_theme_to_windows(&app, &theme);
    Ok(())
}

/// 实时把主题应用到已打开窗口的**原生外观**（`set_theme`）。玻璃/毛玻璃材质跟随窗口
/// NSAppearance，故仅切前端 CSS 还不够；配置实时变更（A12）也需调用此函数同步原生层。
pub(crate) fn apply_theme_to_windows(app: &AppHandle, theme: &str) {
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
    #[cfg(unix)]
    {
        // 路由到统一宿主（全局单窗）；宿主不可用时回退到本进程内建窗。
        route_open_window(app, crate::gui_host::WindowKind::Settings, false, None);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        // Settings window only needs general (theme) to build; the page fetches secret presence via
        // get_settings() separately. Skip keychain here.
        let cfg = AppConfig::load_without_secrets();
        let pin = crate::app::popup_pin(&app, &cfg);
        crate::app::create_settings_window(&app, &cfg, pin).map_err(|e| e.to_string())
    }
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
    /// 已安装但脚本与最新内置版本不一致 → 需更新。
    outdated: bool,
    hooks_json_exists: bool,
    supported: bool,
}

#[tauri::command]
pub fn cursor_hook_status() -> HookStatus {
    HookStatus {
        installed: cursor_hook::is_installed(),
        outdated: cursor_hook::needs_update(),
        hooks_json_exists: cursor_hook::hooks_json_exists(),
        supported: cursor_hook::supported(),
    }
}

#[tauri::command]
pub fn cursor_hook_install() -> Result<String, String> {
    cursor_hook::install().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cursor_hook_update() -> Result<String, String> {
    cursor_hook::update().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cursor_hook_uninstall() -> Result<String, String> {
    cursor_hook::uninstall().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cursor_hook_reveal() {
    cursor_hook::reveal();
}

// ===== Claude Code Hook（PreToolUse 超时延长） =====

use crate::integrations::claude_hook;

/// Claude Code Hook 安装状态（与 Cursor Hook 对称，驱动设置页徽标与按钮）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeHookStatus {
    installed: bool,
    /// 已安装但脚本与最新内置版本不一致 → 需更新。
    outdated: bool,
    settings_exists: bool,
    supported: bool,
}

#[tauri::command]
pub fn claude_hook_status() -> ClaudeHookStatus {
    ClaudeHookStatus {
        installed: claude_hook::is_installed(),
        outdated: claude_hook::needs_update(),
        settings_exists: claude_hook::settings_exists(),
        supported: claude_hook::supported(),
    }
}

#[tauri::command]
pub fn claude_hook_install() -> Result<String, String> {
    claude_hook::install().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn claude_hook_update() -> Result<String, String> {
    claude_hook::update().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn claude_hook_uninstall() -> Result<String, String> {
    claude_hook::uninstall().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn claude_hook_reveal() {
    claude_hook::reveal();
}

// ===== Agent 全局规则（Cursor / Claude Code / Codex） =====

use crate::integrations::agent_rules::{self, AgentTarget};

/// Rules 安装状态（驱动设置页 Agent 分组的徽标与按钮）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleStatus {
    installed: bool,
    /// 已安装但提示词正文与最新内置版本不一致 → 需更新。
    outdated: bool,
    /// 展示用文件路径（home 折叠为 ~）。
    path: String,
    supported: bool,
}

fn parse_agent(agent: &str) -> Result<AgentTarget, String> {
    AgentTarget::parse(agent).ok_or_else(|| {
        crate::i18n::tr(crate::i18n::Lang::current(), "cmd.unknownAgent").to_string()
    })
}

#[tauri::command]
pub fn agent_rule_status(agent: String) -> Result<RuleStatus, String> {
    let a = parse_agent(&agent)?;
    Ok(RuleStatus {
        installed: agent_rules::is_installed(a),
        outdated: agent_rules::needs_update(a),
        path: agent_rules::display_path(a),
        supported: agent_rules::supported(a),
    })
}

#[tauri::command]
pub fn agent_rule_install(agent: String) -> Result<String, String> {
    let a = parse_agent(&agent)?;
    agent_rules::install(a).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn agent_rule_update(agent: String) -> Result<String, String> {
    let a = parse_agent(&agent)?;
    agent_rules::update(a).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn agent_rule_uninstall(agent: String) -> Result<String, String> {
    let a = parse_agent(&agent)?;
    agent_rules::uninstall(a).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn agent_rule_reveal(agent: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    agent_rules::reveal(a);
    Ok(())
}

#[tauri::command]
pub fn agent_rule_open(agent: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    agent_rules::open(a);
    Ok(())
}

// ===== Agent 三态模式（CLI | MCP | 未集成） =====

use crate::integrations::{agent_mode, mcp_config};

/// 某家 Agent 的模式聚合状态（驱动设置页三态分段控件 + 产物清单）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModeStatus {
    /// 当前模式："none" | "cli" | "mcp"。
    mode: String,
    /// 当前模式下是否有产物过期 / 缺失。
    needs_update: bool,
    /// Rule 文件展示路径（home 折叠为 ~）。
    rule_path: String,
    rule_installed: bool,
    /// 该 Agent 是否有「超时 Hook」概念（Codex 没有）。
    timeout_hook_supported: bool,
    timeout_hook_installed: bool,
    /// MCP 配置文件展示路径。
    mcp_config_path: String,
    mcp_config_installed: bool,
}

#[tauri::command]
pub fn agent_mode_status(agent: String) -> Result<AgentModeStatus, String> {
    let a = parse_agent(&agent)?;
    Ok(AgentModeStatus {
        mode: agent_mode::current(a).as_str().to_string(),
        needs_update: agent_mode::needs_update(a),
        rule_path: agent_rules::display_path(a),
        rule_installed: agent_rules::is_installed(a),
        timeout_hook_supported: agent_mode::timeout_hook_supported(a),
        timeout_hook_installed: agent_mode::timeout_hook_is_installed(a),
        mcp_config_path: mcp_config::display_path(a),
        mcp_config_installed: mcp_config::is_installed(a),
    })
}

/// 一键切换到目标模式（"none"|"cli"|"mcp"）：自动卸旧装新。
#[tauri::command]
pub fn agent_mode_set(agent: String, mode: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    let m = agent_mode::Mode::parse(&mode)
        .ok_or_else(|| crate::i18n::tr(crate::i18n::Lang::current(), "cmd.unknownMode").to_string())?;
    agent_mode::set(a, m).map_err(|e| e.to_string())
}

/// 把当前模式的全部产物刷新到最新（不切换模式）。
#[tauri::command]
pub fn agent_mode_update(agent: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    agent_mode::update(a).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn mcp_config_reveal(agent: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    mcp_config::reveal(a);
    Ok(())
}

#[tauri::command]
pub fn mcp_config_open(agent: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    mcp_config::open(a);
    Ok(())
}

/// 当前可执行文件绝对路径，供手动集成卡的 MCP 配置示例直接填入 `command`（与自动集成写入一致）。
#[tauri::command]
pub fn mcp_command_path() -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| crate::i18n::tr(lang, "cmd.locateExeFailed").replace("{e}", &e.to_string()))
}

#[tauri::command]
pub fn agent_hook_reveal(agent: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    agent_mode::timeout_hook_reveal(a);
    Ok(())
}

#[tauri::command]
pub fn agent_hook_open(agent: String) -> Result<(), String> {
    let a = parse_agent(&agent)?;
    agent_mode::timeout_hook_open(a);
    Ok(())
}

// ===== Agent 生命周期追踪 hook（实验性功能） =====

use crate::agents::AgentKind;
use crate::integrations::agent_lifecycle;

fn parse_agent_kind(agent: &str) -> Result<AgentKind, String> {
    AgentKind::parse(agent).ok_or_else(|| {
        crate::i18n::tr(crate::i18n::Lang::current(), "cmd.unknownAgent").to_string()
    })
}

#[tauri::command]
pub fn agent_lifecycle_status(agent: String) -> Result<agent_lifecycle::LifecycleStatus, String> {
    let k = parse_agent_kind(&agent)?;
    Ok(agent_lifecycle::status(k))
}

#[tauri::command]
pub fn agent_lifecycle_install(app: AppHandle, agent: String) -> Result<String, String> {
    let k = parse_agent_kind(&agent)?;
    let msg = agent_lifecycle::install(k).map_err(|e| e.to_string())?;
    refresh_host_tray(&app);
    Ok(msg)
}

#[tauri::command]
pub fn agent_lifecycle_uninstall(app: AppHandle, agent: String) -> Result<String, String> {
    let k = parse_agent_kind(&agent)?;
    let msg = agent_lifecycle::uninstall(k).map_err(|e| e.to_string())?;
    refresh_host_tray(&app);
    Ok(msg)
}

/// 生命周期 hook 装/卸后刷新托盘菜单，使「Agent 状态」入口随之显隐。仅在统一 GUI 宿主进程内
/// （持有 `HostState`）实际生效；其它进程自动 no-op。
fn refresh_host_tray(app: &AppHandle) {
    #[cfg(unix)]
    {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || crate::app::gui_host::refresh_tray(&app2));
    }
    #[cfg(not(unix))]
    {
        let _ = app;
    }
}

// ===== Telegram 测试连接 =====

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramTestArgs {
    pub bot_token: String,
    pub chat_id: String,
    pub api_base_url: String,
}

#[tauri::command]
pub async fn telegram_test(args: TelegramTestArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    let bot_token = fallback_secret(&args.bot_token, |c| c.channels.telegram.bot_token.clone());
    let client = TelegramClient::new(bot_token, args.chat_id, args.api_base_url)
        .map_err(|e| e.localized(lang))?;
    client
        .test_connection(lang)
        .await
        .map_err(|e| e.localized(lang))
}

// ===== 钉钉测试连接 / userId 自动识别 =====

use crate::config::DingTalkChannelConfig;
use crate::dingtalk::client::DingTalkClient;
use crate::dingtalk::stream::{StreamConn, StreamEvent, TOPIC_BOT_MESSAGE};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkTestArgs {
    pub client_id: String,
    pub client_secret: String,
    pub user_id: String,
}

/// 测试连接：换 token（校验 ClientId/Secret）+ 向 userId 单聊发一条测试消息。
#[tauri::command]
pub async fn dingtalk_test(args: DingTalkTestArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    if args.user_id.trim().is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillUserId").to_string());
    }
    let client_secret = fallback_secret(&args.client_secret, |c| {
        c.channels.dingding.client_secret.clone()
    });
    let cfg = DingTalkChannelConfig {
        enabled: true,
        client_id: args.client_id,
        client_secret,
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
    pub client_id: String,
    pub client_secret: String,
}

/// 自动识别准备：校验 ClientId/Secret（换 token），通过后返回供用户私聊发送的 4 位识别码。
/// 校验不通过则返回中文错误（前端据此不展示识别码、不进入等待）。
#[tauri::command]
pub async fn dingtalk_detect_prepare(args: DingTalkDetectArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    let client_id = args.client_id.trim();
    let secret = fallback_secret(&args.client_secret, |c| {
        c.channels.dingding.client_secret.clone()
    });
    let client_secret = secret.trim();
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
    pub client_id: String,
    pub client_secret: String,
    pub code: String,
}

/// 自动识别等待：开 Stream（bot 消息 topic），等到内容等于识别码的单聊消息，返回其 senderStaffId。
/// 120 秒超时报错。
#[tauri::command]
pub async fn dingtalk_detect_wait(args: DingTalkWaitArgs) -> Result<String, String> {
    use std::time::Duration;
    let lang = crate::i18n::Lang::current();
    let client_id = args.client_id.trim().to_string();
    let secret = fallback_secret(&args.client_secret, |c| {
        c.channels.dingding.client_secret.clone()
    });
    let client_secret = secret.trim().to_string();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillClientIdSecret").to_string());
    }
    let code = args.code.trim().to_string();
    if code.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
    }

    detect_with_cancel(lang, async move {
        // Q6：经 Daemon 长连接识别（避免与 Daemon 单连接冲突）。Daemon 接管即用其结果；
        // 接不通 Daemon 才回退进程内临时连接（非 Unix 无 Daemon，直接走回退）。
        #[cfg(unix)]
        {
            let req = crate::ipc::DetectRequest {
                kind: "dingtalk".to_string(),
                app_key: client_id.clone(),
                app_secret: client_secret.clone(),
                base_url: String::new(),
                code: code.clone(),
                lang: lang.code().to_string(),
            };
            if let Some(result) = crate::client::request_detect(req).await {
                return result;
            }
        }

        let http = reqwest::Client::new();
        let mut stream =
            StreamConn::connect(http, &client_id, &client_secret, &[TOPIC_BOT_MESSAGE])
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
                        if let Some(sender) = data.get("senderStaffId").and_then(|v| v.as_str()) {
                            return Ok(sender.to_string());
                        }
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => return Err(crate::i18n::tr(lang, "cmd.streamDisconnected").to_string()),
                Err(_) => return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string()),
            }
        }
    })
    .await
}

// ===== 飞书测试连接 / open_id 自动识别 =====

use crate::config::FeishuChannelConfig;
use crate::feishu::client::FeishuClient;
use crate::feishu::ws::{FeishuWs, WsEvent};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuTestArgs {
    pub app_id: String,
    pub app_secret: String,
    pub open_id: String,
    pub base_url: String,
}

/// 测试连接：换 token（校验 AppId/Secret）+ 向 open_id 单聊发一条测试消息。
#[tauri::command]
pub async fn feishu_test(args: FeishuTestArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    if args.open_id.trim().is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillOpenId").to_string());
    }
    let app_secret = fallback_secret(&args.app_secret, |c| c.channels.feishu.app_secret.clone());
    let cfg = FeishuChannelConfig {
        enabled: true,
        app_id: args.app_id,
        app_secret,
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
    pub app_id: String,
    pub app_secret: String,
    pub base_url: String,
}

/// 自动识别准备：校验 AppId/Secret（换 token），通过后返回供用户私聊发送的 4 位识别码。
#[tauri::command]
pub async fn feishu_detect_prepare(args: FeishuDetectArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    let app_id = args.app_id.trim();
    let secret = fallback_secret(&args.app_secret, |c| c.channels.feishu.app_secret.clone());
    let app_secret = secret.trim();
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
    pub app_id: String,
    pub app_secret: String,
    pub base_url: String,
    pub code: String,
}

/// 自动识别等待：开长连接，等到内容等于识别码的单聊消息，返回发送者 open_id。120 秒超时报错。
#[tauri::command]
pub async fn feishu_detect_wait(args: FeishuWaitArgs) -> Result<String, String> {
    use std::time::Duration;
    let lang = crate::i18n::Lang::current();
    let app_id = args.app_id.trim().to_string();
    let secret = fallback_secret(&args.app_secret, |c| c.channels.feishu.app_secret.clone());
    let app_secret = secret.trim().to_string();
    if app_id.is_empty() || app_secret.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillAppIdSecret").to_string());
    }
    let code = args.code.trim().to_string();
    if code.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
    }
    let base_url = effective_feishu_base(&args.base_url);

    detect_with_cancel(lang, async move {
        // Q6：经 Daemon 长连接识别（见钉钉同段说明）。
        #[cfg(unix)]
        {
            let req = crate::ipc::DetectRequest {
                kind: "feishu".to_string(),
                app_key: app_id.clone(),
                app_secret: app_secret.clone(),
                base_url: base_url.clone(),
                code: code.clone(),
                lang: lang.code().to_string(),
            };
            if let Some(result) = crate::client::request_detect(req).await {
                return result;
            }
        }

        let http = reqwest::Client::new();
        let mut ws = FeishuWs::connect(http, &base_url, &app_id, &app_secret)
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
    })
    .await
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
    let content_str = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let content: serde_json::Value = serde_json::from_str(content_str).ok()?;
    let text = content.get("text").and_then(|v| v.as_str())?.to_string();
    Some((open_id, text))
}

// ===== Slack 测试连接 / userId 自动识别 =====

use crate::config::SlackChannelConfig;
use crate::slack::client::SlackClient;
use crate::slack::ws::{self as slack_ws, SlackWs, WsEvent as SlackWsEvent};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackTestArgs {
    pub bot_token: String,
    pub app_token: String,
    pub user_id: String,
}

/// 测试连接：校验 Bot Token（auth.test + 向 userId 发测试 DM）+ 校验 App Token（apps.connections.open）。
#[tauri::command]
pub async fn slack_test(args: SlackTestArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    if args.user_id.trim().is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillUserId").to_string());
    }
    let bot_token = fallback_secret(&args.bot_token, |c| c.channels.slack.bot_token.clone());
    let app_token = fallback_secret(&args.app_token, |c| c.channels.slack.app_token.clone());
    let cfg = SlackChannelConfig {
        enabled: true,
        bot_token,
        app_token,
        user_id: args.user_id,
    };
    let client = SlackClient::new(&cfg).map_err(|e| e.localized(lang))?;
    // Bot Token：auth.test + 解析 DM + 发测试消息。
    client.auth_test().await.map_err(|e| e.localized(lang))?;
    let dm = client.open_dm().await.map_err(|e| e.localized(lang))?;
    client
        .post_text(&dm, crate::i18n::tr(lang, "cmd.slTestRemote"))
        .await
        .map_err(|e| e.localized(lang))?;
    // App Token：能拿到 wss 即通过（不保持长连）。
    slack_ws::open_socket_url(client.http(), client.app_token())
        .await
        .map_err(|e| e.localized(lang))?;
    Ok(crate::i18n::tr(lang, "cmd.slTestSent").to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackDetectArgs {
    pub bot_token: String,
    pub app_token: String,
}

/// 自动识别准备：校验双 token（App Token 能开 Socket Mode）后返回供用户私聊发送的 4 位识别码。
#[tauri::command]
pub async fn slack_detect_prepare(args: SlackDetectArgs) -> Result<String, String> {
    let lang = crate::i18n::Lang::current();
    let bot_token = fallback_secret(&args.bot_token, |c| c.channels.slack.bot_token.clone());
    let app_token = fallback_secret(&args.app_token, |c| c.channels.slack.app_token.clone());
    if bot_token.trim().is_empty() || app_token.trim().is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillSlackTokens").to_string());
    }
    let http = reqwest::Client::new();
    slack_ws::open_socket_url(&http, app_token.trim())
        .await
        .map_err(|e| e.localized(lang))?;
    Ok(gen_detect_code())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackWaitArgs {
    pub bot_token: String,
    pub app_token: String,
    pub code: String,
}

/// 自动识别等待：开 Socket Mode，等到 DM 文本内容等于识别码的消息，返回发送者 user id。120 秒超时报错。
#[tauri::command]
pub async fn slack_detect_wait(args: SlackWaitArgs) -> Result<String, String> {
    use std::time::Duration;
    let lang = crate::i18n::Lang::current();
    let bot_token = fallback_secret(&args.bot_token, |c| c.channels.slack.bot_token.clone());
    let app_token = fallback_secret(&args.app_token, |c| c.channels.slack.app_token.clone());
    if bot_token.trim().is_empty() || app_token.trim().is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.fillSlackTokens").to_string());
    }
    let code = args.code.trim().to_string();
    if code.is_empty() {
        return Err(crate::i18n::tr(lang, "cmd.detectCodeInvalid").to_string());
    }

    detect_with_cancel(lang, async move {
        // Q6：经 Daemon 长连接识别（见钉钉/飞书同段说明）。app_key=App Token（Socket 复用键），
        // app_secret=Bot Token（建连时校验齐全）。
        #[cfg(unix)]
        {
            let req = crate::ipc::DetectRequest {
                kind: "slack".to_string(),
                app_key: app_token.trim().to_string(),
                app_secret: bot_token.trim().to_string(),
                base_url: String::new(),
                code: code.clone(),
                lang: lang.code().to_string(),
            };
            if let Some(result) = crate::client::request_detect(req).await {
                return result;
            }
        }

        let http = reqwest::Client::new();
        let mut ws = SlackWs::connect(http, app_token.trim())
            .await
            .map_err(|e| e.localized(lang))?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string());
            }
            match tokio::time::timeout(remaining, ws.recv()).await {
                Ok(Some(SlackWsEvent::Message(event))) => {
                    if let Some((user, text)) = slack_text_and_sender(&event) {
                        if text.trim() == code {
                            return Ok(user);
                        }
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => return Err(crate::i18n::tr(lang, "cmd.streamDisconnected").to_string()),
                Err(_) => return Err(crate::i18n::tr(lang, "cmd.detectTimeout").to_string()),
            }
        }
    })
    .await
}

/// 从 Slack message 事件取 (发送者 user id, 文本内容)。无文本返回 None。
fn slack_text_and_sender(event: &serde_json::Value) -> Option<(String, String)> {
    let user = event.get("user").and_then(|v| v.as_str())?.to_string();
    let text = event.get("text").and_then(|v| v.as_str())?.to_string();
    Some((user, text))
}

// ===== 自动识别「取消」支持（三家共用） =====
//
// 识别的「等待」步骤最多阻塞 120s。UI 加了「取消」按钮，但 Tauri 命令本身不可中断，所以这里
// 用一个进程内取消信号：每次 `*_detect_wait` 注册一个新 `Notify`，命令体经 `detect_with_cancel`
// 与 `notified()` 竞速；`detect_cancel` 命令置位即让等待提前返回并 **drop 掉等待 future**——
// 对走 daemon 的路径这会关掉到 daemon 的连接（daemon 侧 `handle_detect` 随之中止并释放临时长连接），
// 对进程内回退路径则直接 drop 临时 WS。UI 同一时刻只有一个识别在跑，故全局单槽即可。

static DETECT_CANCEL: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::Arc<tokio::sync::Notify>>>> =
    std::sync::OnceLock::new();

fn detect_cancel_slot() -> &'static std::sync::Mutex<Option<std::sync::Arc<tokio::sync::Notify>>> {
    DETECT_CANCEL.get_or_init(|| std::sync::Mutex::new(None))
}

/// 为当前识别注册一个新的取消令牌（替换任何旧令牌）。
fn detect_cancel_register() -> std::sync::Arc<tokio::sync::Notify> {
    let token = std::sync::Arc::new(tokio::sync::Notify::new());
    *detect_cancel_slot().lock().unwrap() = Some(token.clone());
    token
}

/// 身份安全地清槽：仅当槽里仍是本次令牌时才清（避免误清后一次识别的令牌）。
fn detect_cancel_clear(token: &std::sync::Arc<tokio::sync::Notify>) {
    let mut guard = detect_cancel_slot().lock().unwrap();
    if matches!(guard.as_ref(), Some(cur) if std::sync::Arc::ptr_eq(cur, token)) {
        *guard = None;
    }
}

/// 跑识别 `work`，直到完成或被取消；取消时返回本地化的「已取消」。
async fn detect_with_cancel<F>(lang: crate::i18n::Lang, work: F) -> Result<String, String>
where
    F: std::future::Future<Output = Result<String, String>>,
{
    let token = detect_cancel_register();
    let out = tokio::select! {
        result = work => result,
        _ = token.notified() => Err(crate::i18n::tr(lang, "cmd.detectCancelled").to_string()),
    };
    detect_cancel_clear(&token);
    out
}

/// 取消正在进行的「自动识别」等待（UI「取消」按钮调用）。无进行中识别则无操作。
#[tauri::command]
pub fn detect_cancel() {
    if let Some(token) = detect_cancel_slot().lock().unwrap().take() {
        token.notify_one();
    }
}

/// 生成 4 位识别码（瞬时配对用，无需强随机）。
fn gen_detect_code() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{:04}", nanos % 10000)
}

// ===== 版本自更新（self-update） =====

/// daemon 经 GUI Helper 推送的自更新态（弹窗进程内缓存，规避「事件早于前端监听」的竞态）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushedUpdateState {
    pub available: bool,
    pub latest_version: String,
    pub pending: bool,
}

static PUSHED_UPDATE: std::sync::OnceLock<std::sync::Mutex<PushedUpdateState>> =
    std::sync::OnceLock::new();

fn pushed_update_slot() -> &'static std::sync::Mutex<PushedUpdateState> {
    PUSHED_UPDATE.get_or_init(|| std::sync::Mutex::new(PushedUpdateState::default()))
}

/// GUI Helper 读到 daemon 的 `UpdateState` 后写入此缓存（供弹窗挂载时拉取初值）。
pub fn set_pushed_update(state: PushedUpdateState) {
    if let Ok(mut slot) = pushed_update_slot().lock() {
        *slot = state;
    }
}

/// 弹窗挂载时拉取「已推送的自更新态」初值（之后变化经事件实时更新）。
#[tauri::command]
pub fn popup_update_state() -> PushedUpdateState {
    pushed_update_slot()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

/// 本地当前版本（编译期嵌入）。
#[tauri::command]
pub fn get_app_version() -> String {
    crate::update::current_version()
}

/// 检查更新：查远端最新正式版并与本地比较。`manual=true` 时清空「忽略」集合。
#[tauri::command]
pub async fn update_check(manual: bool) -> Result<crate::update::UpdateInfo, String> {
    if manual {
        crate::update::state::clear_dismissed();
    }
    let info = crate::update::check().await.map_err(|e| e.to_string())?;
    crate::update::state::record_check(&info.latest_version, &info.release_notes);
    Ok(info)
}

/// 取指定版本（tag `v<version>`）的更新日志（关于区「查看当前版本更新日志」用）。
#[tauri::command]
pub async fn update_get_version_notes(version: String) -> Result<String, String> {
    crate::update::notes::notes_for_tag(&version)
        .await
        .map_err(|e| e.to_string())
}

/// 取更新日志：`aggregate=true` 聚合「当前版本→最新版本」之间所有版本（懒加载）。
#[tauri::command]
pub async fn update_get_notes(aggregate: bool) -> Result<String, String> {
    if !aggregate {
        return crate::update::notes::latest_notes()
            .await
            .map_err(|e| e.to_string());
    }
    let current = crate::update::current_version();
    let to = {
        let st = crate::update::state::load();
        if st.latest_version.is_empty() {
            crate::update::check()
                .await
                .map_err(|e| e.to_string())?
                .latest_version
        } else {
            st.latest_version
        }
    };
    crate::update::notes::aggregated_notes(&current, &to)
        .await
        .map_err(|e| e.to_string())
}

/// 应用更新：把新二进制落盘（不 restart；换新交给 daemon drain）。下载进度经
/// `update_download_progress` 事件回传；完成发 `update_apply_finished`。
#[tauri::command]
pub async fn update_apply(app: AppHandle) -> Result<(), String> {
    let updater = crate::update::select_updater();
    let app_for_cb = app.clone();
    let cb: crate::update::ProgressCb = Box::new(move |p| {
        let _ = app_for_cb.emit("update_download_progress", p);
    });
    updater.apply(Some(cb)).await.map_err(|e| e.to_string())?;
    crate::update::state::set_pending(true);
    let _ = app.emit("update_apply_finished", ());
    Ok(())
}

/// 忽略某版本（不再主动弹该版本提示；设置内手动检查可重置）。
#[tauri::command]
pub fn update_dismiss(version: String) {
    crate::update::state::dismiss(&version);
}

/// 设置内更新后「重启设置页面」：用新二进制重开设置进程，再退出当前设置窗。
#[tauri::command]
pub fn restart_settings(app: AppHandle) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let lang = crate::i18n::Lang::current();
    let exe = std::env::current_exe()
        .map_err(|e| crate::i18n::tr(lang, "cmd.locateExeFailed").replace("{e}", &e.to_string()))?;
    Command::new(exe)
        .arg("--settings")
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| crate::i18n::tr(lang, "cmd.openFailed").replace("{e}", &e.to_string()))?;
    app.exit(0);
    Ok(())
}
