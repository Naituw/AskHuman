//! Pane transitions freeze a native viewport; normal edge drags use native autoresizing.
use super::popup_inbox_geometry::{Allocation, Animation, Frame, Transition};
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
    type AnimateDone = extern "C" fn(*mut std::ffi::c_void, *const f64, i32);

    unsafe extern "C" {
        fn ah_preview_screen_x() -> f64;
        fn ah_collapse_animate(
            webview: *mut std::ffi::c_void,
            window: *mut std::ffi::c_void,
            json: *const std::ffi::c_char,
            done: AnimateDone,
            user: *mut std::ffi::c_void,
        );
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

    extern "C" fn animated(user: *mut std::ffi::c_void, values: *const f64, count: i32) {
        // Ownership of the boxed sender comes back exactly once per animation.
        let sender = unsafe { Box::from_raw(user as *mut oneshot::Sender<Vec<f64>>) };
        let report = if values.is_null() || count <= 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(values, count as usize) }.to_vec()
        };
        let _ = sender.send(report);
    }

    /// Collapse, expand or resize the small window with the display-link driver (PopupCollapse.swift).
    /// The window keeps its top-left corner (plus the clamped shift in `allocation.frame`), the pinned
    /// viewport stays where it is on screen, and without an animation the change is a single frame.
    pub async fn animate(
        window: &WebviewWindow,
        allocation: &Allocation,
        transition: Transition,
        source: Frame,
        animation: Option<Animation>,
    ) -> Result<Frame, String> {
        let canvas = allocation.canvas.clone().ok_or("missing native canvas")?;
        let duration = animation
            .map(|a| a.duration_ms)
            .filter(|n| n.is_finite())
            .map_or(0.0, |n| n.clamp(0.0, 2000.0));
        let curve = animation
            .and_then(|a| a.curve)
            .filter(|c| c.iter().all(|n| n.is_finite()))
            .unwrap_or([0.2, 0.0, 0.0, 1.0]);
        let (lights, lights_start, lights_end) = match transition {
            Transition::Expand => ("show", 0.45, 1.0),
            Transition::Collapse => ("hide", 0.0, 0.35),
            // The small window stays without traffic lights; this only makes sure they are gone.
            _ => ("settle", 0.0, 0.4),
        };
        let expand = transition == Transition::Expand;
        let params = serde_json::json!({
            "toWidth": allocation.frame.width,
            "toHeight": allocation.frame.height,
            // Relative to the frame the driver starts from, so a move since `source` is kept.
            "toDX": allocation.frame.x - source.x,
            "canvasX1": allocation.left_span() - canvas.left,
            "durationMs": duration,
            "curve": curve,
            "lights": lights,
            "lightsStart": lights_start,
            "lightsEnd": lights_end,
            // The small window cannot be resized by its edges.
            "resizable": expand,
            // Expanding unpins the viewport and gives the window back the minimum size it was
            // created with; `finish` then sets the minimum of the current layout.
            "resume": expand,
            "minWidth": super::super::popup_size::MIN_WIDTH,
            "minHeight": super::super::popup_size::MIN_HEIGHT,
        });
        let json = std::ffi::CString::new(params.to_string()).map_err(|e| e.to_string())?;
        let (tx, rx) = oneshot::channel::<Vec<f64>>();
        let user = Box::into_raw(Box::new(tx)) as usize;
        let scheduled = window.with_webview(move |platform| unsafe {
            ah_collapse_animate(
                platform.inner(),
                platform.ns_window(),
                json.as_ptr(),
                animated,
                user as *mut std::ffi::c_void,
            );
        });
        if let Err(error) = scheduled {
            // The closure never ran, so the sender is still ours to drop.
            drop(unsafe { Box::from_raw(user as *mut oneshot::Sender<Vec<f64>>) });
            return Err(error.to_string());
        }
        let wait = std::time::Duration::from_millis(duration as u64 + 5000);
        let values = tokio::time::timeout(wait, rx)
            .await
            .map_err(|_| "window animation timed out".to_string())?
            .map_err(|_| "window animation callback dropped".to_string())?;
        // Report layout (PopupCollapse.swift): 5 finalX, 6 finalTop, 7 finalW, 8 finalH, 9 cancelled.
        if values.len() < 10 {
            return Err("window animation returned no report".into());
        }
        if values[9] != 0.0 {
            return Err("window animation was interrupted".into());
        }
        let screen_height = unsafe { CGDisplayPixelsHigh(CGMainDisplayID()) } as f64;
        Ok(Frame {
            x: values[5],
            y: screen_height - values[6],
            width: values[7],
            height: values[8],
        })
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
        instant::apply(window, allocation, popup_scale(window))
    }
}
/// Change the window form (small window <-> expanded layout). Windows and Linux have no pinned
/// viewport to keep still, so the change is instant there: the minimum size is lowered for the small
/// window and its edges stop resizing; expanding gives the edges back (`resume` restores the minimum).
pub async fn animate(
    window: &WebviewWindow,
    allocation: &Allocation,
    transition: Transition,
    source: Frame,
    animation: Option<Animation>,
) -> Result<Frame, String> {
    #[cfg(target_os = "macos")]
    return mac::animate(window, allocation, transition, source, animation).await;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (source, animation);
        instant::animate(window, allocation, transition, popup_scale(window))
    }
}
pub async fn resume(window: &WebviewWindow, allocation: &Allocation) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::resume(window, allocation).await;
    #[cfg(not(target_os = "macos"))]
    instant::resume(window, allocation, popup_scale(window))
}

#[cfg(target_os = "linux")]
fn popup_scale(window: &WebviewWindow) -> f64 {
    super::linux_dpi::page_scale(window).get()
}

#[cfg(target_os = "windows")]
fn popup_scale(_: &WebviewWindow) -> f64 {
    1.0
}

pub(super) fn platform_logical_size(
    size: (f64, f64),
    page_scale: f64,
    uses_page_scale: bool,
) -> Option<(f64, f64)> {
    if !size.0.is_finite() || !size.1.is_finite() || size.0 <= 0.0 || size.1 <= 0.0 {
        return None;
    }
    super::popup_size::platform_window_size(size, page_scale, uses_page_scale)
}

fn platform_logical_position(position: (f64, f64)) -> Option<(f64, f64)> {
    if !position.0.is_finite() || !position.1.is_finite() {
        return None;
    }
    (position.0.abs() <= i32::MAX as f64 && position.1.abs() <= i32::MAX as f64).then_some(position)
}

/// The window changes of the platforms without a pinned viewport (Windows, Linux): the page's viewport is the
/// window, so every change is made at once. Only cross-platform window calls are used and the module is not gated by
/// `cfg`, so a macOS build type-checks it as well even though macOS never calls it.
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod instant {
    use super::*;
    use tauri::{LogicalPosition, LogicalSize};

    pub fn apply(
        window: &WebviewWindow,
        allocation: &Allocation,
        page_scale: f64,
    ) -> Result<Frame, String> {
        apply_frame(window, allocation.frame, page_scale)?;
        Ok(allocation.frame)
    }

    /// A shrink lowers the minimum size first (the small window is smaller than the normal minimum), then sizes the
    /// window, then takes its edges away; an expansion gives the edges back first so that the sizing is not refused.
    pub fn animate(
        window: &WebviewWindow,
        allocation: &Allocation,
        transition: Transition,
        page_scale: f64,
    ) -> Result<Frame, String> {
        let expand = transition == Transition::Expand;
        if expand {
            window.set_resizable(true).map_err(|e| e.to_string())?;
        } else {
            window
                .set_min_size(Some(tauri::LogicalSize::new(1.0, 1.0)))
                .map_err(|e| e.to_string())?;
        }
        apply_frame(window, allocation.frame, page_scale)?;
        if !expand {
            window.set_resizable(false).map_err(|e| e.to_string())?;
        }
        Ok(allocation.frame)
    }

    pub fn resume(
        window: &WebviewWindow,
        allocation: &Allocation,
        page_scale: f64,
    ) -> Result<(), String> {
        let minimum = platform_logical_size(
            (
                allocation.minimum_width(),
                super::super::popup_size::MIN_HEIGHT.min(allocation.frame.height),
            ),
            page_scale,
            cfg!(target_os = "linux"),
        )
        .ok_or("Popup window has an invalid page scale")?;
        window
            .set_min_size(Some(LogicalSize::new(minimum.0, minimum.1)))
            .map_err(|e| e.to_string())
    }

    fn apply_frame(window: &WebviewWindow, plan: Frame, page_scale: f64) -> Result<(), String> {
        let (width, height) = platform_logical_size(
            (plan.width, plan.height),
            page_scale,
            cfg!(target_os = "linux"),
        )
        .ok_or("Popup window has an invalid page scale")?;
        window
            .set_size(LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        let (x, y) = platform_logical_position((plan.x, plan.y))
            .ok_or("Popup window has an invalid position")?;
        let positioned = window
            .set_position(LogicalPosition::new(x, y))
            .map_err(|e| e.to_string());
        // A Wayland compositor places windows itself, so a refused position is not an error there.
        if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Ok(());
        }
        positioned
    }
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
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn css_frame_and_minimum_convert_to_tao_logical_size() {
        for page_scale in [1.25, 1.5] {
            let frame = Frame {
                x: -1280.0,
                y: 200.0,
                width: 806.0,
                height: 620.0,
            };
            assert_eq!(
                platform_logical_size((frame.width, frame.height), page_scale, true),
                Some((806.0 * page_scale, 620.0 * page_scale))
            );
            assert_eq!(
                platform_logical_size((1132.0, 480.0), page_scale, true),
                Some((1132.0 * page_scale, 480.0 * page_scale))
            );
            assert_eq!(
                platform_logical_position((frame.x, frame.y)),
                Some((frame.x, frame.y))
            );
            let round_trip = platform_logical_position((frame.x, frame.y)).unwrap();
            assert_eq!(round_trip, (frame.x, frame.y));
            assert_eq!(
                platform_logical_size((frame.width, frame.height), page_scale, false),
                Some((frame.width, frame.height))
            );
            assert_eq!(
                platform_logical_position((frame.x, frame.y)),
                Some((frame.x, frame.y))
            );
        }
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(platform_logical_size((806.0, 620.0), invalid, true), None);
        }
    }
}
