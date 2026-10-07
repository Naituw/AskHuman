//! Pane transitions freeze a native viewport; normal edge drags use native autoresizing.
use super::popup_inbox_geometry::{Allocation, Frame};
use tauri::WebviewWindow;

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::NSRect;
    use std::sync::OnceLock;
    use tokio::sync::oneshot;
    struct ServerBounds {
        connection: unsafe extern "C" fn() -> i32,
        bounds: unsafe extern "C" fn(i32, u32, *mut NSRect) -> i32,
    }
    fn server_bounds() -> Option<&'static ServerBounds> {
        static API: OnceLock<Option<ServerBounds>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let handle = libc::dlopen(
                c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
                libc::RTLD_LAZY,
            );
            if handle.is_null() {
                return None;
            }
            let connection = libc::dlsym(handle, c"SLSMainConnectionID".as_ptr());
            let bounds = libc::dlsym(handle, c"SLSGetWindowBounds".as_ptr());
            if connection.is_null() || bounds.is_null() {
                return None;
            }
            Some(ServerBounds {
                connection: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn() -> i32,
                >(connection),
                bounds: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(i32, u32, *mut NSRect) -> i32,
                >(bounds),
            })
        })
        .as_ref()
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Sample {
        window_frame: [f64; 4],
        canvas_x: f64,
        model_x: f64,
        expected_x: f64,
        server_x: Option<f64>,
        presentation_x: Option<f64>,
        viewport_width: f64,
        viewport_height: f64,
        preview_x: Option<f64>,
    }
    unsafe fn measure(
        platform: &tauri::webview::PlatformWebview,
        left: f64,
        anchor: f64,
    ) -> Sample {
        let native = platform.ns_window() as *mut AnyObject;
        let webview = platform.inner() as *mut AnyObject;
        let canvas: *mut AnyObject = msg_send![webview, superview];
        let frame: NSRect = msg_send![native, frame];
        let canvas_frame: NSRect = msg_send![canvas, frame];
        let viewport: NSRect = msg_send![webview, frame];
        let server_x = server_bounds().and_then(|api| {
            let number: i64 = msg_send![native, windowNumber];
            let mut bounds = NSRect::ZERO;
            ((api.bounds)((api.connection)(), number as u32, &mut bounds) == 0)
                .then_some(bounds.origin.x + canvas_frame.origin.x + left)
        });
        let layer: *mut AnyObject = msg_send![canvas, layer];
        let presentation: *mut AnyObject = msg_send![layer, presentationLayer];
        let presentation_x = if presentation.is_null() {
            None
        } else {
            let bounds: NSRect = msg_send![presentation, frame];
            server_x.map(|x| x - canvas_frame.origin.x + bounds.origin.x)
        };
        let preview_x = ah_preview_screen_x();
        Sample {
            window_frame: [
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ],
            canvas_x: canvas_frame.origin.x,
            model_x: frame.origin.x + canvas_frame.origin.x + left,
            expected_x: anchor,
            server_x,
            presentation_x,
            viewport_width: viewport.size.width,
            viewport_height: viewport.size.height,
            preview_x: preview_x.is_finite().then_some(preview_x),
        }
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayPixelsHigh(display: u32) -> usize;
    }
    unsafe extern "C" {
        fn ah_preview_screen_x() -> f64;
        fn ah_popup_canvas_prepare(
            view: *mut std::ffi::c_void,
            left: f64,
            width: f64,
            height: f64,
            visible_left: f64,
        );
        fn ah_popup_canvas_position(view: *mut std::ffi::c_void, x: f64, height: f64);
        fn ah_popup_canvas_resume(
            view: *mut std::ffi::c_void,
            left: f64,
            visible_left: f64,
            minimum_width: f64,
            minimum_height: f64,
        );
    }
    pub async fn prepare(
        window: &WebviewWindow,
        allocation: &Allocation,
        old_left: f64,
    ) -> Result<(), String> {
        let canvas = allocation.canvas.clone().ok_or("missing native canvas")?;
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                ah_popup_canvas_prepare(
                    platform.inner(),
                    canvas.left,
                    canvas.width,
                    canvas.height,
                    old_left,
                );
                let _ = tx.send(());
            })
            .map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "canvas preparation disconnected".into())
    }
    pub async fn resume(window: &WebviewWindow, allocation: &Allocation) -> Result<(), String> {
        let canvas = allocation.canvas.clone().ok_or("missing native canvas")?;
        let left = allocation.left_span();
        let minimum = allocation.minimum_width();
        let minimum_height = super::super::popup_size::MIN_HEIGHT.min(allocation.frame.height);
        let review = crate::dev_instance::is_dev_instance()
            && std::env::var("ASKHUMAN_INBOX_LAYOUT_REVIEW").as_deref() == Ok("1");
        let plan = allocation.clone();
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let before = review.then(|| measure(&platform, canvas.left, plan.frame.x + left));
                ah_popup_canvas_resume(
                    platform.inner(),
                    canvas.left,
                    left,
                    minimum,
                    minimum_height,
                );
                if let Some(before) = before {
                    use std::io::Write;
                    let after = measure(&platform, canvas.left, plan.frame.x + left);
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(crate::paths::state_dir().join("popup-canvas-resume-review.jsonl"))
                    {
                        let _ = writeln!(
                            file,
                            "{}",
                            serde_json::json!({"plan":plan,"before":before,"after":after})
                        );
                    }
                }
                let _ = tx.send(());
            })
            .map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "canvas resumption disconnected".into())
    }
    pub async fn apply(
        window: &WebviewWindow,
        allocation: &Allocation,
        old_left: f64,
        source: Frame,
    ) -> Result<Frame, String> {
        let plan = allocation.clone();
        let canvas = allocation.canvas.clone().ok_or("missing native canvas")?;
        let new_left = allocation.left_span();
        let review = crate::dev_instance::is_dev_instance()
            && std::env::var("ASKHUMAN_INBOX_LAYOUT_REVIEW").as_deref() == Ok("1");
        let (tx, rx) = oneshot::channel();
        window.with_webview(move |platform| unsafe {
            let native = platform.ns_window() as *mut AnyObject;
            let from: NSRect = msg_send![native, frame];
            let screen_height = CGDisplayPixelsHigh(CGMainDisplayID()) as f64;
            let current = Frame { x: from.origin.x, y: screen_height - from.origin.y - from.size.height,
                width: from.size.width, height: from.size.height };
            let mut target = super::rebase_target(current, source, plan.frame);
            let scale: f64 = msg_send![native, backingScaleFactor];
            let from_anchor = current.x + old_left;
            let mut anchor = target.x + new_left;
            if (anchor - from_anchor).abs() <= 1.0 / scale {
                anchor = from_anchor;
                target.x = anchor - new_left;
            }
            let frame = NSRect::new(objc2_foundation::NSPoint::new(target.x, screen_height - target.y - target.height),
                objc2_foundation::NSSize::new(target.width, target.height));
            let transaction = AnyClass::get(c"CATransaction").unwrap();
            let _: () = msg_send![transaction, begin];
            let _: () = msg_send![transaction, setDisableActions: true];
            let _: () = msg_send![native, setFrame: frame, display: false];
            let actual: NSRect = msg_send![native, frame];
            ah_popup_canvas_position(platform.inner(), anchor - actual.origin.x - canvas.left, canvas.height);
            let _: () = msg_send![native, displayIfNeeded];
            let _: () = msg_send![transaction, commit];
            let _: () = msg_send![transaction, flush];
            if review {
                use std::io::Write;
                let dir = crate::paths::state_dir();
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("popup-canvas-review.jsonl")) {
                    let sample = measure(&platform, canvas.left, anchor);
                    let _ = writeln!(file, "{}", serde_json::json!({"revision":plan.revision,"durationMs":0,"samples":[sample],
                        "from":[from.origin.x,from.origin.y,from.size.width,from.size.height],"source":source,"plan":plan}));
                }
            }
            let _ = tx.send(Frame { x: actual.origin.x, y: screen_height - actual.origin.y - actual.size.height,
                width: actual.size.width, height: actual.size.height });
        }).map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "canvas geometry disconnected".into())
    }
}

pub async fn prepare(
    window: &WebviewWindow,
    allocation: &Allocation,
    old_left: f64,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::prepare(window, allocation, old_left).await;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, allocation, old_left);
        Ok(())
    }
}
pub async fn apply(
    window: &WebviewWindow,
    allocation: &Allocation,
    old_left: f64,
    source: Frame,
) -> Result<Frame, String> {
    #[cfg(target_os = "macos")]
    return mac::apply(window, allocation, old_left, source).await;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (old_left, source);
        apply_frame(window, allocation.frame)?;
        Ok(allocation.frame)
    }
}
pub async fn resume(window: &WebviewWindow, allocation: &Allocation) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::resume(window, allocation).await;
    #[cfg(not(target_os = "macos"))]
    window
        .set_min_size(Some(tauri::LogicalSize::new(
            allocation.minimum_width(),
            super::popup_size::MIN_HEIGHT.min(allocation.frame.height),
        )))
        .map_err(|e| e.to_string())
}
#[cfg(not(target_os = "macos"))]
fn apply_frame(window: &WebviewWindow, plan: Frame) -> Result<(), String> {
    window
        .set_size(tauri::LogicalSize::new(plan.width, plan.height))
        .map_err(|e| e.to_string())?;
    let positioned = window
        .set_position(tauri::LogicalPosition::new(plan.x, plan.y))
        .map_err(|e| e.to_string());
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return Ok(());
    }
    positioned
}

#[cfg(any(target_os = "macos", test))]
fn rebase_target(current: Frame, source: Frame, target: Frame) -> Frame {
    // Pane dimensions are absolute; only position follows a user move during preparation.
    Frame {
        x: target.x + current.x - source.x,
        y: target.y + current.y - source.y,
        ..target
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_later_resize_cannot_compound_the_absolute_pane_size() {
        let source = Frame {
            x: 1000.0,
            y: 200.0,
            width: 560.0,
            height: 620.0,
        };
        let current = Frame {
            width: 590.0,
            height: 650.0,
            ..source
        };
        let target = Frame {
            x: 754.0,
            width: 806.0,
            ..source
        };
        let result = rebase_target(current, source, target);
        assert_eq!((result.width, result.height), (806.0, 620.0));
        assert_eq!((result.x, result.y), (754.0, 200.0));
    }
    #[test]
    fn a_later_move_preserves_the_answer_anchor_and_vertical_position() {
        let source = Frame {
            x: 1000.0,
            y: 200.0,
            width: 560.0,
            height: 620.0,
        };
        let current = Frame {
            x: 1300.5,
            y: 350.5,
            ..source
        };
        let target = Frame {
            x: 754.0,
            width: 806.0,
            ..source
        };
        let result = rebase_target(current, source, target);
        assert_eq!(result.x + 246.0, current.x);
        assert_eq!(result.y, current.y);
        assert_eq!((result.width, result.height), (806.0, 620.0));
    }
}
