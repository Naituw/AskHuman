//! The popup takes the front when files are dragged over it, before the web view reads them.
//!
//! macOS only opens files dragged from some places (the Dock's stacks, for one) for a drop target that is the
//! frontmost application when it first asks for them. wry asks in its `draggingEntered:` handler, so the
//! application has to be frontmost by then; see `swift/PopupDragFront.swift`.

use tauri::WebviewWindow;

/// Registers the popup's web view for the hook. A failure only costs the behavior, never the drop itself.
pub fn install(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn ah_popup_drag_front_install(web_view: *mut std::ffi::c_void) -> i32;
        }
        let scheduled = window.with_webview(|platform| {
            // SAFETY: `inner` is the live WKWebView, and `with_webview` runs this closure on the main thread.
            if unsafe { ah_popup_drag_front_install(platform.inner()) } == 0 {
                log_not_installed("hook_install_failed");
            }
        });
        if scheduled.is_err() {
            log_not_installed("with_webview_failed");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Records a failed install in `daemon.log`: the popup host's stderr goes nowhere, and a wry upgrade that renames the
/// web view class or its drag methods would otherwise bring the bug back unnoticed. Fixed labels only.
#[cfg(target_os = "macos")]
fn log_not_installed(reason: &str) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    eprintln!("[askhuman] popup drag-front hook not installed: {reason}");
    crate::daemon::lifecycle::append_log_line(&format!(
        r#"{{"timestampMs":{timestamp_ms},"pid":{},"event":"askhuman_drag_front","installed":false,"reason":"{reason}"}}"#,
        std::process::id()
    ));
}
