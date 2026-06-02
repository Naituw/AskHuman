//! 前端可调用的 Tauri 命令（弹窗模式）。

use crate::app::{self, AppState};
use crate::config::ThemeMode;
use crate::models::{AskRequest, ChannelAction, ChannelResult, ImageAttachment};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

/// 弹窗初始化负载：请求内容 + 主题（前端据此套用样式）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupInit {
    request: AskRequest,
    theme: String,
}

#[tauri::command]
pub fn popup_init(state: State<AppState>) -> PopupInit {
    PopupInit {
        request: state.request.clone(),
        theme: theme_str(state.config.general.theme),
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
    app::finish(&app, result);
}

#[tauri::command]
pub fn cancel_popup(app: AppHandle) {
    app::finish(&app, ChannelResult::cancel("popup"));
}

fn theme_str(theme: ThemeMode) -> String {
    match theme {
        ThemeMode::System => "system",
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
    }
    .to_string()
}
