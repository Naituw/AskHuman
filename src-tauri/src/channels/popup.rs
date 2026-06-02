//! 本地弹窗 Channel：窗口在 `app::launch` 的 setup 中创建，结果经 IPC 命令进入协调器。
//! 此处主要负责被抢答时关闭窗口。

use super::{Channel, ResultSink};
use crate::models::AskRequest;
use tauri::{AppHandle, Manager};

pub struct PopupChannel {
    app: AppHandle,
}

impl PopupChannel {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Channel for PopupChannel {
    fn id(&self) -> &str {
        "popup"
    }

    fn start(&self, _request: &AskRequest, _sink: ResultSink) {
        // 窗口已由 setup 创建；用户操作经 submit_popup / cancel_popup 命令进入协调器。
    }

    fn cancel_by_other(&self) {
        if let Some(w) = self.app.get_webview_window("popup") {
            let _ = w.close();
        }
    }
}
