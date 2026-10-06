//! Isolated geometry experiment: one fixed Vue canvas, native clipping, no product IPC or snapshots.
use serde::Serialize;
use std::{io::Write, sync::Mutex, time::Instant};
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const LEFT: f64 = 241.0;
const MAIN: f64 = 560.0;
const RIGHT: f64 = 401.0;
const CANVAS: f64 = LEFT + MAIN + RIGHT;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Edges {
    left: f64,
    right: f64,
}
fn interpolate(from: Edges, to: Edges, progress: f64, scale: f64) -> Edges {
    let t = progress.clamp(0.0, 1.0);
    let eased = t * t * t * (10.0 + t * (-15.0 + 6.0 * t));
    let pixel = |a: f64, b: f64| ((a + (b - a) * eased) * scale).round() / scale;
    Edges {
        left: pixel(from.left, to.left),
        right: pixel(from.right, to.right),
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Probe {
    main_x: f64,
    expected_main_x: f64,
    server_main_x: Option<f64>,
    presentation_main_x: Option<f64>,
    left_line_x: f64,
    right_line_x: f64,
    webview_width: f64,
    webview_height: f64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    frames: usize,
    max_model_drift: f64,
    max_server_drift: Option<f64>,
    max_presentation_drift: Option<f64>,
    max_viewport_drift: f64,
    max_gap_ms: f64,
    final_probe: Probe,
}
struct State {
    #[cfg(target_os = "macos")]
    base: Mutex<Option<mac::Base>>,
    edges: Mutex<Edges>,
    automatic: bool,
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::NSRect;
    use std::{ffi::c_void, sync::OnceLock};
    use tokio::sync::oneshot;
    #[derive(Clone, Copy)]
    pub struct Base {
        frame: NSRect,
        view: NSRect,
        scale: f64,
        anchor: f64,
    }
    struct Server {
        connection: unsafe extern "C" fn() -> i32,
        bounds: unsafe extern "C" fn(i32, u32, *mut NSRect) -> i32,
    }
    fn server() -> Option<&'static Server> {
        static API: OnceLock<Option<Server>> = OnceLock::new();
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
            Some(Server {
                connection: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> i32>(
                    connection,
                ),
                bounds: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(i32, u32, *mut NSRect) -> i32,
                >(bounds),
            })
        })
        .as_ref()
    }
    unsafe fn server_x(window: *mut AnyObject) -> Option<f64> {
        let api = server()?;
        let number: i64 = msg_send![window, windowNumber];
        let mut rect = NSRect::ZERO;
        ((api.bounds)((api.connection)(), number as u32, &mut rect) == 0).then_some(rect.origin.x)
    }
    unsafe fn measure(native: *mut AnyObject, view: *mut AnyObject, base: Base) -> Probe {
        let frame: NSRect = msg_send![native, frame];
        let view_frame: NSRect = msg_send![view, frame];
        let layer: *mut AnyObject = msg_send![view, layer];
        let presentation: *mut AnyObject = if layer.is_null() {
            std::ptr::null_mut()
        } else {
            msg_send![layer, presentationLayer]
        };
        let server_x = server_x(native);
        let model = frame.origin.x + view_frame.origin.x + LEFT;
        let presentation_main_x = if presentation.is_null() {
            None
        } else {
            let rect: NSRect = msg_send![presentation, frame];
            server_x.map(|x| x + rect.origin.x + LEFT)
        };
        Probe {
            main_x: model,
            expected_main_x: base.anchor,
            server_main_x: server_x.map(|x| x + view_frame.origin.x + LEFT),
            presentation_main_x,
            left_line_x: model - 1.0,
            right_line_x: model + MAIN,
            webview_width: view_frame.size.width,
            webview_height: view_frame.size.height,
        }
    }
    pub async fn init(window: &WebviewWindow) -> Result<Base, String> {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let native = platform.ns_window() as *mut AnyObject;
                let view = platform.inner() as *mut AnyObject;
                let frame: NSRect = msg_send![native, frame];
                let original: NSRect = msg_send![view, frame];
                let base = Base {
                    frame,
                    view: original,
                    scale,
                    anchor: frame.origin.x + original.origin.x,
                };
                let _: () = msg_send![view, setAutoresizingMask: 0usize];
                let mut canvas = original;
                canvas.origin.x -= LEFT;
                canvas.size.width = CANVAS;
                let _: () = msg_send![view, setFrame: canvas];
                let parent: *mut AnyObject = msg_send![view, superview];
                let _: () = msg_send![parent, setWantsLayer: true];
                let layer: *mut AnyObject = msg_send![parent, layer];
                let _: () = msg_send![layer, setMasksToBounds: true];
                let _ = tx.send(base);
            })
            .map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "native initialization disconnected".into())
    }
    pub async fn refresh(
        window: &WebviewWindow,
        mut base: Base,
        edges: Edges,
    ) -> Result<Base, String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let native = platform.ns_window() as *mut AnyObject;
                let frame: NSRect = msg_send![native, frame];
                base.frame.origin.x = frame.origin.x + edges.left;
                base.frame.origin.y = frame.origin.y;
                base.anchor = base.frame.origin.x + base.view.origin.x;
                let _ = tx.send(base);
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "native anchor disconnected".into())
    }
    pub fn scale(base: Base) -> f64 {
        base.scale
    }
    pub fn height(base: Base) -> f64 {
        base.view.size.height
    }
    pub async fn apply(
        window: &WebviewWindow,
        base: Base,
        edges: Option<Edges>,
    ) -> Result<Probe, String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let native = platform.ns_window() as *mut AnyObject;
                let view = platform.inner() as *mut AnyObject;
                if let Some(edges) = edges {
                    let transaction = AnyClass::get(c"CATransaction").unwrap();
                    let _: () = msg_send![transaction, begin];
                    let _: () = msg_send![transaction, setDisableActions: true];
                    let mut frame = base.frame;
                    frame.origin.x -= edges.left;
                    frame.size.width += edges.left + edges.right;
                    let _: () = msg_send![native, setFrame: frame, display: false];
                    // Compensate from the actual native frame, on its backing pixel grid. The
                    // WebView's width/height never change, and the DOM never receives a per-frame IPC.
                    let actual: NSRect = msg_send![native, frame];
                    let mut canvas = base.view;
                    canvas.origin.x += base.frame.origin.x - actual.origin.x - LEFT;
                    canvas.size.width = CANVAS;
                    let _: () = msg_send![view, setFrame: canvas];
                    let _: () = msg_send![native, displayIfNeeded];
                    let _: () = msg_send![transaction, commit];
                    let _: () = msg_send![transaction, flush];
                }
                let _ = tx.send(measure(native, view, base));
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "native frame disconnected".into())
    }
}

#[tauri::command]
async fn foundation_ready(
    window: WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        let base = mac::init(&window).await?;
        *state.base.lock().unwrap() = Some(base);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        window
            .set_size(tauri::LogicalSize::new(CANVAS, 600.0))
            .map_err(|e| e.to_string())?;
    }
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(cfg!(target_os = "macos"))
}
#[tauri::command]
async fn foundation_mode(
    window: WebviewWindow,
    state: tauri::State<'_, State>,
    baseline: bool,
    left: bool,
    right: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let previous = *state.edges.lock().unwrap();
        let base = state.base.lock().unwrap().ok_or("demo is not ready")?;
        let base = mac::refresh(&window, base, previous).await?;
        *state.base.lock().unwrap() = Some(base);
        let edges = Edges {
            left: if baseline || left { LEFT } else { 0.0 },
            right: if baseline || right { RIGHT } else { 0.0 },
        };
        mac::apply(&window, base, Some(edges)).await?;
        *state.edges.lock().unwrap() = edges;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, state, baseline, left, right);
    }
    Ok(())
}
#[tauri::command]
async fn foundation_animate(
    window: WebviewWindow,
    state: tauri::State<'_, State>,
    left: bool,
    right: bool,
    baseline: bool,
    duration_ms: u64,
) -> Result<Report, String> {
    #[cfg(target_os = "macos")]
    {
        let from = *state.edges.lock().unwrap();
        let base = state.base.lock().unwrap().ok_or("demo is not ready")?;
        let base = mac::refresh(&window, base, from).await?;
        *state.base.lock().unwrap() = Some(base);
        let to = Edges {
            left: if left { LEFT } else { 0.0 },
            right: if right { RIGHT } else { 0.0 },
        };
        let initial = mac::apply(&window, base, None).await?;
        let mut report = Report {
            frames: 0,
            max_model_drift: 0.0,
            max_server_drift: initial.server_main_x.map(|_| 0.0),
            max_presentation_drift: initial.presentation_main_x.map(|_| 0.0),
            max_viewport_drift: 0.0,
            max_gap_ms: 0.0,
            final_probe: initial,
        };
        let started = Instant::now();
        let mut previous = started;
        loop {
            let now = Instant::now();
            report.max_gap_ms = report
                .max_gap_ms
                .max(now.duration_since(previous).as_secs_f64() * 1000.0);
            previous = now;
            let progress = (now.duration_since(started).as_secs_f64() * 1000.0
                / duration_ms.clamp(80, 5000) as f64)
                .min(1.0);
            let edges = interpolate(from, to, progress, mac::scale(base));
            // Sample the rendered presentation layer before advancing the next native frame.
            let probe = mac::apply(&window, base, None).await?;
            let drift = |x: f64| (x - probe.expected_main_x).abs();
            report.frames += 1;
            report.max_model_drift = report.max_model_drift.max(drift(probe.main_x));
            if let Some(x) = probe.server_main_x {
                report.max_server_drift =
                    Some(report.max_server_drift.unwrap_or(0.0).max(drift(x)));
            }
            if let Some(x) = probe.presentation_main_x {
                report.max_presentation_drift =
                    Some(report.max_presentation_drift.unwrap_or(0.0).max(drift(x)));
            }
            report.max_viewport_drift = report
                .max_viewport_drift
                .max((probe.webview_width - CANVAS).abs())
                .max((probe.webview_height - mac::height(base)).abs());
            report.final_probe = probe;
            if !baseline {
                mac::apply(&window, base, Some(edges)).await?;
            }
            if progress >= 1.0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }
        if !baseline {
            *state.edges.lock().unwrap() = to;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        report.final_probe = mac::apply(&window, base, None).await?;
        let final_probe = &report.final_probe;
        report.max_model_drift = report
            .max_model_drift
            .max((final_probe.main_x - final_probe.expected_main_x).abs());
        if let Some(x) = final_probe.server_main_x {
            report.max_server_drift = Some(
                report
                    .max_server_drift
                    .unwrap_or(0.0)
                    .max((x - final_probe.expected_main_x).abs()),
            );
        }
        if let Some(x) = final_probe.presentation_main_x {
            report.max_presentation_drift = Some(
                report
                    .max_presentation_drift
                    .unwrap_or(0.0)
                    .max((x - final_probe.expected_main_x).abs()),
            );
        }
        report.max_viewport_drift = report
            .max_viewport_drift
            .max((final_probe.webview_width - CANVAS).abs())
            .max((final_probe.webview_height - mac::height(base)).abs());
        Ok(report)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, state, left, right, baseline, duration_ms);
        Err("native crop experiment is macOS-only".into())
    }
}
#[tauri::command]
fn foundation_exit(app: tauri::AppHandle) {
    app.exit(0);
}
#[tauri::command]
fn foundation_record(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    report: serde_json::Value,
    finished: bool,
) -> Result<(), String> {
    let path = std::env::var("ASKHUMAN_FOUNDATION_LOG").map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{report}").map_err(|e| e.to_string())?;
    if finished && state.automatic {
        app.exit(if report["pass"].as_bool().unwrap_or(false) {
            0
        } else {
            1
        });
    }
    Ok(())
}
fn main() {
    let automatic = std::env::var_os("ASKHUMAN_FOUNDATION_AUTO").is_some();
    tauri::Builder::default()
        .manage(State {
            #[cfg(target_os = "macos")]
            base: Mutex::new(None),
            edges: Mutex::new(Edges {
                left: 0.0,
                right: 0.0,
            }),
            automatic,
        })
        .invoke_handler(tauri::generate_handler![
            foundation_ready,
            foundation_mode,
            foundation_animate,
            foundation_exit,
            foundation_record
        ])
        .setup(move |app| {
            let mut builder = WebviewWindowBuilder::new(
                app,
                "pane-foundation",
                WebviewUrl::App(if automatic {
                    "prototype/pane-foundation.html?auto=1".into()
                } else {
                    "prototype/pane-foundation.html".into()
                }),
            )
            .title("三栏基础 Demo")
            .inner_size(MAIN, 600.0)
            .position(500.0, 120.0)
            .visible(false)
            .resizable(false);
            #[cfg(target_os = "macos")]
            {
                builder = builder
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .hidden_title(true);
            }
            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("foundation demo failed");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_edges_share_the_backing_pixel_grid_without_moving_the_middle() {
        for scale in [1.0, 2.0, 3.0] {
            for (from, to) in [
                (
                    Edges {
                        left: 0.0,
                        right: 0.0,
                    },
                    Edges {
                        left: LEFT,
                        right: RIGHT,
                    },
                ),
                (
                    Edges {
                        left: LEFT,
                        right: 0.0,
                    },
                    Edges {
                        left: 0.0,
                        right: RIGHT,
                    },
                ),
            ] {
                for step in 0..1001 {
                    let edges = interpolate(from, to, step as f64 / 1000.0, scale);
                    let window_x = 500.0 - edges.left;
                    let canvas_x = edges.left - LEFT;
                    assert!((window_x + canvas_x + LEFT - 500.0).abs() < 1e-9);
                    assert!((edges.left * scale).fract().abs() < 1e-9);
                    assert!((edges.right * scale).fract().abs() < 1e-9);
                    assert!((window_x + canvas_x + LEFT + MAIN - 1060.0).abs() < 1e-9);
                }
            }
        }
    }
}
