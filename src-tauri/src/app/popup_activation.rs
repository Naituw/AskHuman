//! Whether the Popup Host is the active application, for the "peek" rule of the shared popup page: an expanded
//! window that shows only requests put aside shrinks back to the small window when the user leaves AskHuman
//! (docs/specs/popup-inbox-collapse.md C23, docs/plans/popup-inbox-collapse.md 2.5).
//!
//! macOS reports the application's activation (NSApplication notifications). Unlike a window's key state it is not
//! disturbed by the sheets, file panels and Quick Look panel of this process. Elsewhere the popup window's focus
//! stands in for it; the page's short grace period absorbs focus moving between windows of the process.

use tauri::{AppHandle, WebviewWindow};

/// Event carrying a `bool`: the process became (`true`) or stopped being (`false`) the active application.
const EVENT: &str = "popup-app-active";

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::OnceLock;
    use tauri::{AppHandle, Emitter};

    static APP: OnceLock<AppHandle> = OnceLock::new();

    unsafe extern "C" {
        fn ah_popup_activation_observe(changed: extern "C" fn(i32)) -> i32;
    }

    extern "C" fn changed(active: i32) {
        if let Some(app) = APP.get() {
            let _ = app.emit(super::EVENT, active != 0);
        }
    }

    /// Main thread only: starts reporting once and says whether the process is active right now.
    pub fn observe(app: &AppHandle) -> bool {
        let _ = APP.set(app.clone());
        unsafe { ah_popup_activation_observe(changed) != 0 }
    }
}

/// Starts reporting changes (idempotent) and returns whether this process is the active application right now.
/// The page asks once after it listens; later changes arrive as `popup-app-active` events.
pub async fn start(app: &AppHandle, window: &WebviewWindow) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = app.clone();
        app.run_on_main_thread(move || {
            let _ = tx.send(platform::observe(&handle));
        })
        .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        window_is_focused(window)
    }
}

/// Windows and Linux: whether the popup window has the focus right now (the start state of the page's rule).
/// Like `window_focus`, this is not gated by `cfg`, so a macOS build type-checks it too.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn window_is_focused(window: &WebviewWindow) -> Result<bool, String> {
    window.is_focused().map_err(|e| e.to_string())
}

/// Windows and Linux: the popup window gaining or losing the focus is the activation signal.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn window_focus(app: &AppHandle, focused: bool) {
    use tauri::Emitter;
    let _ = app.emit(EVENT, focused);
}
