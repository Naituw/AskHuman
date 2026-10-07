//! Nonactivating window presentation, native appearance and legacy pulse diagnostics.
use tauri::WebviewWindow;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontProbe {
    own_pid: i32,
    front_pid_before: i32,
    front_pid_after: i32,
    key_before: i64,
    key_after: i64,
    restored: bool,
    minimized_after: bool,
    visible_before: bool,
    appear_behavior_applied: Option<isize>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PulseProbe {
    pub(super) original_width: f64,
    pub(super) peak_width: f64,
    pub(super) final_width: f64,
    pub(super) original_x: f64,
    pub(super) final_x: f64,
    pub(super) animated: bool,
    pub(super) frame_stable: bool,
    pub(super) transform_restored: bool,
    pub(super) frames: u32,
    pub(super) peak_scale: f64,
    pub(super) max_gap_ms: f64,
    pub(super) peak_readback: bool,
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use tokio::sync::oneshot;
    unsafe fn foreground_pid() -> i32 {
        let workspace: *mut AnyObject =
            msg_send![AnyClass::get(c"NSWorkspace").unwrap(), sharedWorkspace];
        let front: *mut AnyObject = msg_send![workspace, frontmostApplication];
        if front.is_null() {
            0
        } else {
            msg_send![front, processIdentifier]
        }
    }

    unsafe fn key_number() -> i64 {
        let app: *mut AnyObject =
            msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
        let key: *mut AnyObject = msg_send![app, keyWindow];
        if key.is_null() {
            0
        } else {
            msg_send![key, windowNumber]
        }
    }

    pub async fn front(
        window: &WebviewWindow,
        appear_behavior: Option<isize>,
    ) -> Result<FrontProbe, String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let native_window = platform.ns_window() as *mut AnyObject;
                let before = foreground_pid();
                let key = key_number();
                let minimized: bool = msg_send![native_window, isMiniaturized];
                let visible: bool = msg_send![native_window, isVisible];
                let mut applied = None;
                if minimized {
                    let _: () =
                        msg_send![native_window, deminiaturize: std::ptr::null_mut::<AnyObject>()];
                }
                if !visible && !minimized {
                    if let Some(behavior) = appear_behavior {
                        let _: () = msg_send![native_window, setAnimationBehavior: behavior];
                        applied = Some(msg_send![native_window, animationBehavior]);
                        // orderFront: starts AppKit's configured appearance without making
                        // the window key. The following raise also crosses inactive app order.
                        let _: () =
                            msg_send![native_window, orderFront: std::ptr::null_mut::<AnyObject>()];
                    }
                }
                // This raises an inactive application's window without making it key or main.
                // Do not use Tauri show/set_focus or activate the application in this path.
                let _: () = msg_send![native_window, orderFrontRegardless];
                let after: bool = msg_send![native_window, isMiniaturized];
                let _ = tx.send(FrontProbe {
                    own_pid: std::process::id() as i32,
                    front_pid_before: before,
                    front_pid_after: foreground_pid(),
                    key_before: key,
                    key_after: key_number(),
                    restored: minimized,
                    minimized_after: after,
                    visible_before: visible,
                    appear_behavior_applied: applied,
                });
            })
            .map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "foreground callback disconnected".to_string())
    }
}

/// Apply the configured native animation when a new round reveals a hidden window.
/// Existing visible windows and minimized restoration retain their normal raise path.
#[cfg(target_os = "macos")]
pub async fn appear(window: &WebviewWindow, behavior: isize) -> Result<FrontProbe, String> {
    mac::front(window, Some(behavior)).await
}

pub async fn front(window: &WebviewWindow) -> Result<FrontProbe, String> {
    #[cfg(target_os = "macos")]
    {
        mac::front(window, None).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let w = window.clone();
        window
            .run_on_main_thread(move || {
                let result = (|| {
                    #[cfg(target_os = "windows")]
                    {
                        use windows_sys::Win32::UI::WindowsAndMessaging::{
                            SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE,
                            SWP_NOSIZE, SW_SHOWNOACTIVATE,
                        };
                        let hwnd = w.hwnd().map_err(|e| e.to_string())?.0;
                        // Keep Tao's visibility flags in sync before the native raise. Popup
                        // windows are built with focused(false), so this show is nonactivating.
                        // Native ShowWindow alone leaves hide() as a cached hidden-to-hidden no-op.
                        w.show().map_err(|e| e.to_string())?;
                        unsafe {
                            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                            SetWindowPos(
                                hwnd,
                                HWND_TOP,
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                        }
                    }
                    #[cfg(target_os = "linux")]
                    {
                        use gio::glib::object::ObjectType;
                        #[link(name = "gtk-3")]
                        unsafe extern "C" {
                            fn gtk_window_set_focus_on_map(
                                window: *mut std::ffi::c_void,
                                focus: i32,
                            );
                            fn gtk_widget_show(widget: *mut std::ffi::c_void);
                            fn gtk_window_deiconify(window: *mut std::ffi::c_void);
                        }
                        let gtk = w.gtk_window().map_err(|e| e.to_string())?;
                        unsafe {
                            let pointer = gtk.as_ptr() as *mut std::ffi::c_void;
                            gtk_window_set_focus_on_map(pointer, 0);
                            gtk_widget_show(pointer);
                            gtk_window_deiconify(pointer);
                        }
                        use raw_window_handle::{
                            HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
                        };
                        #[link(name = "X11")]
                        unsafe extern "C" {
                            fn XRaiseWindow(
                                display: *mut std::ffi::c_void,
                                window: std::os::raw::c_ulong,
                            ) -> i32;
                            fn XFlush(display: *mut std::ffi::c_void) -> i32;
                        }
                        if let (Ok(d), Ok(h)) = (w.display_handle(), w.window_handle()) {
                            if let (RawDisplayHandle::Xlib(d), RawWindowHandle::Xlib(h)) =
                                (d.as_raw(), h.as_raw())
                            {
                                if let Some(display) = d.display {
                                    unsafe {
                                        XRaiseWindow(display.as_ptr(), h.window);
                                        XFlush(display.as_ptr());
                                    }
                                }
                            }
                        }
                    }
                    Ok(FrontProbe {
                        own_pid: std::process::id() as i32,
                        front_pid_before: 0,
                        front_pid_after: 0,
                        key_before: 0,
                        key_after: 0,
                        restored: false,
                        minimized_after: false,
                        visible_before: false,
                        appear_behavior_applied: None,
                    })
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())?
    }
}
