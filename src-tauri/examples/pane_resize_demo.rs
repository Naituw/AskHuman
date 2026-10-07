//! Isolated normal/frozen layout handoff, using the same native canvas as the popup.
use serde::Serialize;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Probe {
    window_width: f64,
    viewport_width: f64,
    viewport_height: f64,
    canvas_x: f64,
    main_x: f64,
    preview_x: Option<f64>,
    preview_width: Option<f64>,
}
#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;
    static PREVIEW: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" {
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
    pub async fn perform(
        window: &WebviewWindow,
        action: String,
        left: f64,
        main: f64,
        right: f64,
        width: f64,
        height: f64,
    ) -> Result<Probe, String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let native = platform.ns_window() as *mut AnyObject;
                let view = platform.inner() as *mut AnyObject;
                let transaction = AnyClass::get(c"CATransaction").unwrap();
                let _: () = msg_send![transaction, begin];
                let _: () = msg_send![transaction, setDisableActions: true];
                if action == "freeze" {
                    ah_popup_canvas_prepare(
                        platform.inner(),
                        606.0,
                        606.0 + main + 1306.0,
                        height,
                        left,
                    );
                } else if action == "resize" || action == "pane" {
                    let mut frame: NSRect = msg_send![native, frame];
                    if action == "pane" {
                        frame.origin.x = 1100.0 - left;
                    }
                    frame.origin.y += frame.size.height - height;
                    frame.size = NSSize::new(width, height);
                    let _: () = msg_send![native,setFrame:frame,display:false];
                    if action == "pane" {
                        let actual: NSRect = msg_send![native, frame];
                        ah_popup_canvas_position(
                            platform.inner(),
                            1100.0 - actual.origin.x - 606.0,
                            height,
                        );
                    }
                } else if action == "resume" {
                    ah_popup_canvas_resume(platform.inner(), 606.0, left, 420.0 + left, 480.0);
                }
                let canvas: *mut AnyObject = msg_send![view, superview];
                // A native PDFView sibling catches mismatches that a pure HTML panel cannot.
                let mut preview = PREVIEW.load(Ordering::Relaxed) as *mut AnyObject;
                if right > 0.0 {
                    if preview.is_null() {
                        let cls = AnyClass::get(c"PDFView").unwrap();
                        preview = msg_send![cls, alloc];
                        preview = msg_send![preview,initWithFrame:NSRect::ZERO];
                        let _: () = msg_send![canvas,addSubview:preview];
                        if let Ok(path) = std::env::var("ASKHUMAN_RESIZE_PDF") {
                            let path = NSString::from_str(&path);
                            let url: *mut AnyObject =
                                msg_send![AnyClass::get(c"NSURL").unwrap(),fileURLWithPath:&*path];
                            let document: *mut AnyObject =
                                msg_send![AnyClass::get(c"PDFDocument").unwrap(), alloc];
                            let document: *mut AnyObject = msg_send![document,initWithURL:url];
                            if !document.is_null() {
                                let _: () = msg_send![preview,setDocument:document];
                                let _: () = msg_send![preview,setAutoScales:true];
                                let _: () = msg_send![document, release];
                            }
                        }
                        PREVIEW.store(preview as usize, Ordering::Relaxed);
                        let _: () = msg_send![preview, release];
                    }
                    if action == "freeze" || action == "resume" || action == "place" {
                        let rect = NSRect::new(
                            NSPoint::new(606.0 + main + 6.0, 80.0),
                            NSSize::new(right, (height - 80.0).max(0.0)),
                        );
                        let _: () = msg_send![preview,setFrame:rect];
                    }
                    let _: () = msg_send![preview,setHidden:false];
                    if action == "freeze" {
                        let _: () = msg_send![preview,setAutoresizingMask:0usize];
                    }
                    if action == "resume" {
                        let _: () = msg_send![preview,setAutoresizingMask:18usize];
                    }
                } else if !preview.is_null() {
                    let _: () = msg_send![preview,setHidden:true];
                }
                let frame: NSRect = msg_send![native, frame];
                let viewport: NSRect = msg_send![view, frame];
                let canvas_frame: NSRect = msg_send![canvas, frame];
                let preview_frame: Option<NSRect> = if right > 0.0 && !preview.is_null() {
                    Some(msg_send![preview, frame])
                } else {
                    None
                };
                let _: () = msg_send![native, displayIfNeeded];
                let _: () = msg_send![transaction, commit];
                let _: () = msg_send![transaction, flush];
                let _ = tx.send(Probe {
                    window_width: frame.size.width,
                    viewport_width: viewport.size.width,
                    viewport_height: viewport.size.height,
                    canvas_x: canvas_frame.origin.x,
                    main_x: frame.origin.x + canvas_frame.origin.x + 606.0,
                    preview_x: preview_frame.map(|p| p.origin.x),
                    preview_width: preview_frame.map(|p| p.size.width),
                });
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())
    }
}
#[tauri::command]
async fn resize_demo(
    window: WebviewWindow,
    action: String,
    left: f64,
    main: f64,
    right: f64,
    width: f64,
    height: f64,
) -> Result<Probe, String> {
    #[cfg(target_os = "macos")]
    {
        let result =
            mac::perform(&window, action.clone(), left, main, right, width, height).await?;
        if action == "resume" {
            let first_show = !window.is_visible().unwrap_or(false);
            window.show().map_err(|e| e.to_string())?;
            if first_show {
                window.set_focus().map_err(|e| e.to_string())?;
            }
        }
        Ok(result)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, action, left, main, right, width, height);
        Err("macOS-only experiment".into())
    }
}
#[tauri::command]
fn resize_record(
    app: tauri::AppHandle,
    report: serde_json::Value,
    finished: bool,
) -> Result<(), String> {
    use std::io::Write;
    let path = std::env::var("ASKHUMAN_RESIZE_LOG").map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{report}").map_err(|e| e.to_string())?;
    if finished {
        app.exit(if report["pass"].as_bool().unwrap_or(false) {
            0
        } else {
            1
        });
    }
    Ok(())
}
#[tauri::command]
fn resize_open() -> Result<(), String> {
    let path = std::env::var("ASKHUMAN_RESIZE_PDF").map_err(|e| e.to_string())?;
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
fn main() {
    let automatic = std::env::var_os("ASKHUMAN_RESIZE_AUTO").is_some();
    let polish = std::env::var_os("ASKHUMAN_RESIZE_POLISH").is_some();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            resize_demo,
            resize_record,
            resize_open
        ])
        .setup(move |app| {
            let builder = WebviewWindowBuilder::new(
                app,
                "pane-resize",
                WebviewUrl::App(if automatic {
                    "prototype/pane-resize.html?auto=1".into()
                } else if polish {
                    "prototype/pane-resize.html?polish=1".into()
                } else {
                    "prototype/pane-resize.html".into()
                }),
            )
            .title(if polish {
                "统一窗口交互 Demo"
            } else {
                "窗口缩放布局验证"
            })
            .inner_size(560.0, 620.0)
            .position(1100.0, 160.0)
            .visible(false)
            .resizable(true);
            #[cfg(target_os = "macos")]
            let builder = builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true);
            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("resize demo failed");
}
