//! Keep the old WebView pixels anchored while the native frame and DOM change independently.
//! This is a mock-prototype experiment, isolated from the product popup runtime.
use super::Layout;
use tauri::WebviewWindow;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    screen_x: f64,
    expected_screen_x: f64,
    width: f64,
    expected_width: f64,
}

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
    use block2::RcBlock;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::NSRect;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    };
    use tokio::sync::oneshot;

    struct Frozen {
        id: u64,
        image_view: usize,
        window_frame: NSRect,
        view_frame: NSRect,
        screen_frame: NSRect,
        flipped: bool,
    }
    static CURRENT: OnceLock<Mutex<Option<Frozen>>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(1);
    fn current() -> &'static Mutex<Option<Frozen>> {
        CURRENT.get_or_init(|| Mutex::new(None))
    }

    // All Objective-C view operations, including releasing our alloc/init ownership, run on
    // the AppKit thread. Stored integer handles are never dereferenced on worker threads.
    unsafe fn clear(id: Option<u64>) {
        let mut slot = current().lock().unwrap();
        if id.is_some_and(|id| slot.as_ref().is_none_or(|f| f.id != id)) {
            return;
        }
        if let Some(frozen) = slot.take() {
            let view = frozen.image_view as *mut AnyObject;
            let _: () = msg_send![view, removeFromSuperview];
            let _: () = msg_send![view, release];
        }
    }

    pub async fn begin(window: &WebviewWindow) -> Result<bool, String> {
        if !window.is_visible().map_err(|e| e.to_string())? {
            return Ok(false);
        }
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(tx)));
        window.with_webview(move |platform| unsafe {
            clear(None);
            let webview = platform.inner() as *mut AnyObject;
            let native_window = platform.ns_window() as *mut AnyObject;
            let root: *mut AnyObject = msg_send![webview, superview];
            let frame: NSRect = msg_send![webview, frame];
            let window_frame: NSRect = msg_send![native_window, frame];
            let bounds: NSRect = msg_send![webview, bounds];
            let in_window: NSRect = msg_send![webview, convertRect: bounds, toView: std::ptr::null_mut::<AnyObject>()];
            let screen_frame: NSRect = msg_send![native_window, convertRectToScreen: in_window];
            let flipped: bool = msg_send![root, isFlipped];
            // Keep the parent alive if a reset destroys the window before snapshot completion.
            let _: *mut AnyObject = msg_send![root, retain];
            let parent_handle = root as usize;
            let callback = RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
                let root = parent_handle as *mut AnyObject;
                if let Some(tx) = sender.lock().unwrap().take() {
                    if !tx.is_closed() {
                        if image.is_null() {
                            let _ = tx.send(Err("WebView snapshot failed".to_string()));
                        } else {
                            let cls = AnyClass::get(c"NSImageView").unwrap();
                            let view: *mut AnyObject = msg_send![cls, alloc];
                            let view: *mut AnyObject = msg_send![view, initWithFrame: frame];
                            let _: () = msg_send![view, setImage: image];
                            let _: () = msg_send![view, setImageScaling: 2isize];
                            let _: () = msg_send![view, setAutoresizingMask: 0usize];
                            let _: () = msg_send![root, addSubview: view, positioned: 1isize, relativeTo: std::ptr::null_mut::<AnyObject>()];
                            let _: () = msg_send![view, displayIfNeeded];
                            *current().lock().unwrap() = Some(Frozen {
                                id, image_view: view as usize, window_frame, view_frame: frame, screen_frame, flipped,
                            });
                            if tx.send(Ok(true)).is_err() { clear(Some(id)); }
                        }
                    }
                }
                let _: () = msg_send![root, release];
            });
            // WKSnapshotConfiguration defaults to afterScreenUpdates=YES. The image stays in
            // memory and is never exported, written to disk, or used as an answer editor.
            let _: () = msg_send![webview, takeSnapshotWithConfiguration: std::ptr::null_mut::<AnyObject>(), completionHandler: &*callback];
        }).map_err(|e| e.to_string())?;
        let captured = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .map_err(|_| "WebView snapshot timed out".to_string())?
            .map_err(|_| "snapshot callback disconnected".to_string())??;
        // A frontend exception or reset must never leave a frozen image over the editor.
        let watchdog_window = window.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let _ = watchdog_window.run_on_main_thread(move || unsafe {
                clear(Some(id));
            });
        });
        Ok(captured)
    }

    pub async fn apply(window: &WebviewWindow, plan: Layout) -> Result<(), String> {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let old_position = window
            .outer_position()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(scale);
        let old_size = window
            .inner_size()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(scale);
        let (tx, rx) = oneshot::channel();
        window.with_webview(move |platform| unsafe {
            let native_window = platform.ns_window() as *mut AnyObject;
            let old_frame: NSRect = msg_send![native_window, frame];
            let mut frame = old_frame;
            frame.origin.x += plan.x - old_position.x;
            frame.origin.y += old_position.y - plan.y + old_size.height - plan.height;
            frame.size.width += plan.width - old_size.width;
            frame.size.height += plan.height - old_size.height;
            let context = AnyClass::get(c"NSAnimationContext").unwrap();
            let group = RcBlock::new(move |animation: *mut AnyObject| {
                let _: () = msg_send![animation, setDuration: 0.0f64];
                if let Some(frozen) = current().lock().unwrap().as_ref() {
                    let view = frozen.image_view as *mut AnyObject;
                    let mut mask = frozen.view_frame;
                    mask.origin.x += frozen.window_frame.origin.x - frame.origin.x;
                    let shift_y = frozen.window_frame.origin.y - frame.origin.y;
                    mask.origin.y += if frozen.flipped { -shift_y } else { shift_y };
                    let _: () = msg_send![view, setFrame: mask];
                }
                // Resize and move in one AppKit frame update, with the mask compensated in
                // the same group. No intermediate size-only or position-only frame is exposed.
                let _: () = msg_send![native_window, setFrame: frame, display: false];
            });
            let _: () = msg_send![context, runAnimationGroup: &*group, completionHandler: std::ptr::null::<block2::Block<dyn Fn()>>()];
            let _ = tx.send(());
        }).map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "native geometry callback disconnected".to_string())
    }

    pub async fn end(window: &WebviewWindow) -> Result<(), String> {
        let id = current().lock().unwrap().as_ref().map(|f| f.id);
        let Some(id) = id else {
            return Ok(());
        };
        let (tx, rx) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(tx)));
        window.with_webview(move |platform| unsafe {
            let webview = platform.inner() as *mut AnyObject;
            let callback = RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
                // A second after-screen-updates snapshot is a WebKit commit barrier. Merely
                // awaiting Vue nextTick or two JS animation frames does not provide that barrier.
                clear(Some(id));
                if let Some(tx) = sender.lock().unwrap().take() {
                    let result = if image.is_null() { Err("updated WebView snapshot failed".to_string()) } else { Ok(()) };
                    let _ = tx.send(result);
                }
            });
            let _: () = msg_send![webview, takeSnapshotWithConfiguration: std::ptr::null_mut::<AnyObject>(), completionHandler: &*callback];
        }).map_err(|e| e.to_string())?;
        match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
            Ok(result) => result.map_err(|_| "paint callback disconnected".to_string())?,
            Err(_) => {
                let _ = window.run_on_main_thread(move || unsafe {
                    clear(Some(id));
                });
                Err("paint acknowledgment timed out".into())
            }
        }
    }

    pub fn release(window: &WebviewWindow) {
        let _ = window.run_on_main_thread(|| unsafe {
            clear(None);
        });
    }
    pub fn active() -> bool {
        current().lock().unwrap().is_some()
    }

    pub async fn probe(window: &WebviewWindow) -> Result<Option<Probe>, String> {
        let (tx, rx) = oneshot::channel();
        window.with_webview(move |platform| unsafe {
            let result = current().lock().unwrap().as_ref().map(|frozen| {
                let view = frozen.image_view as *mut AnyObject;
                let native_window = platform.ns_window() as *mut AnyObject;
                let bounds: NSRect = msg_send![view, bounds];
                let in_window: NSRect = msg_send![view, convertRect: bounds, toView: std::ptr::null_mut::<AnyObject>()];
                let screen: NSRect = msg_send![native_window, convertRectToScreen: in_window];
                Probe { screen_x: screen.origin.x, expected_screen_x: frozen.screen_frame.origin.x,
                    width: screen.size.width, expected_width: frozen.screen_frame.size.width }
            });
            let _ = tx.send(result);
        }).map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "mask probe disconnected".to_string())
    }

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

    pub async fn front(window: &WebviewWindow) -> Result<FrontProbe, String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let native_window = platform.ns_window() as *mut AnyObject;
                let before = foreground_pid();
                let key = key_number();
                let minimized: bool = msg_send![native_window, isMiniaturized];
                if minimized {
                    let _: () =
                        msg_send![native_window, deminiaturize: std::ptr::null_mut::<AnyObject>()];
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
                });
            })
            .map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "foreground callback disconnected".to_string())
    }
}

pub async fn begin(window: &WebviewWindow) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        mac::begin(window).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok(false)
    }
}
pub async fn apply(window: &WebviewWindow, plan: Layout) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        mac::apply(window, plan).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        window
            .set_size(tauri::LogicalSize::new(plan.width, plan.height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(tauri::LogicalPosition::new(plan.x, plan.y))
            .map_err(|e| e.to_string())
    }
}
pub async fn end(window: &WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        mac::end(window).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok(())
    }
}
pub fn release(window: &WebviewWindow) {
    super::inbox_demo_pulse::cancel();
    #[cfg(target_os = "macos")]
    mac::release(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}
pub fn active() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::active()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

pub async fn probe(window: &WebviewWindow) -> Result<Option<Probe>, String> {
    #[cfg(target_os = "macos")]
    {
        mac::probe(window).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok(None)
    }
}

pub async fn front(window: &WebviewWindow) -> Result<FrontProbe, String> {
    #[cfg(target_os = "macos")]
    {
        mac::front(window).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err("non-activating foreground prototype requires macOS".into())
    }
}
