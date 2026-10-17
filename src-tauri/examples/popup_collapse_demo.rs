//! Native, mock-only prototype for the collapsible inbox window
//! (docs/specs/popup-inbox-collapse.md). It never connects to the product daemon, IM channels or
//! the user's configuration. The page owns the mock inbox state; this file owns the real NSWindow:
//! materials, always-on-top, and the display-link driven frame animation in `swift/PopupCollapse.swift`.
use serde::Serialize;
use std::{
    io::Write,
    sync::atomic::{AtomicI32, Ordering},
};
use tauri::{
    window::{Effect, EffectState, EffectsBuilder},
    Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

const POPUP: &str = "collapse-demo";
const CONTROLS: &str = "collapse-controls";
static TEST_EXIT_CODE: AtomicI32 = AtomicI32::new(2);

struct DemoState {
    /// Any automatic run: `demo_record(finished: true)` ends the process with the recorded verdict.
    automatic: bool,
    /// Which scripted scenario runs inside the answer window (`auto=1` checks, `auto=burst` frame capture,
    /// `auto=video` transitions for an outside screen recording).
    scenario: Option<String>,
    /// Scripted checks never take the keyboard focus: the process refuses to activate itself and windows are shown
    /// without being focused. `--focus` (ASKHUMAN_DEMO_FOCUS) opts out for the one check that needs a real app switch.
    silent: bool,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Info {
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    scale: f64,
    screen_x: f64,
    screen_top: f64,
    screen_w: f64,
    screen_h: f64,
    level: i64,
    always_on_top: bool,
    min_w: f64,
    min_h: f64,
    lights_hidden: bool,
    resizable: bool,
    canvas_installed: bool,
    canvas_responsive: bool,
    canvas_x: f64,
    view_w: f64,
    view_h: f64,
    visible: bool,
    key: bool,
    lights_alpha: f64,
    has_shadow: bool,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Report {
    frames: f64,
    max_gap_ms: f64,
    mean_gap_ms: f64,
    dropped: f64,
    wall_ms: f64,
    final_x: f64,
    final_top: f64,
    final_w: f64,
    final_h: f64,
    cancelled: bool,
    max_error_px: f64,
    refresh_hz: f64,
    canvas_w: f64,
    canvas_h: f64,
    from_x: f64,
    from_top: f64,
    from_w: f64,
    from_h: f64,
    p95_gap_ms: f64,
    start_delay_ms: f64,
    max_cost_ms: f64,
    mean_cost_ms: f64,
    /// Per tick (first 80): gap since the previous tick, callback start and placement end after the vsync (ms).
    gaps: Vec<f64>,
    lates: Vec<f64>,
    ends: Vec<f64>,
    /// Display time (ms, mach clock) each tick's frame is meant for.
    targets: Vec<f64>,
    /// Window size placed by each tick (points).
    widths: Vec<f64>,
    heights: Vec<f64>,
}

impl Report {
    fn from_values(v: &[f64]) -> Report {
        let at = |i: usize| v.get(i).copied().unwrap_or(0.0);
        // Layout after the scalars: tick count, one reserved slot, then six series of `ticks` values each
        // (gaps carry `ticks - 1` values, so the first tick has gap 0).
        let ticks = at(22) as usize;
        let series = |k: usize| -> Vec<f64> {
            let start = if k == 0 {
                24
            } else {
                24 + ticks.saturating_sub(1) + (k - 1) * ticks
            };
            let len = if k == 0 {
                ticks.saturating_sub(1)
            } else {
                ticks
            };
            let mut out: Vec<f64> = v.iter().skip(start).take(len).copied().collect();
            if k == 0 && ticks > 0 {
                out.insert(0, 0.0);
            }
            out
        };
        Report {
            frames: at(0),
            max_gap_ms: at(1),
            mean_gap_ms: at(2),
            dropped: at(3),
            wall_ms: at(4),
            final_x: at(5),
            final_top: at(6),
            final_w: at(7),
            final_h: at(8),
            cancelled: at(9) > 0.5,
            max_error_px: at(10),
            refresh_hz: at(11),
            canvas_w: at(12),
            canvas_h: at(13),
            from_x: at(14),
            from_top: at(15),
            from_w: at(16),
            from_h: at(17),
            p95_gap_ms: at(18),
            start_delay_ms: at(19),
            max_cost_ms: at(20),
            mean_cost_ms: at(21),
            gaps: series(0),
            lates: series(1),
            ends: series(2),
            targets: series(3),
            widths: series(4),
            heights: series(5),
        }
    }
}

/// Pixel comparison of one screen-space region in two window captures.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Diff {
    mean_abs: f64,
    changed_fraction: f64,
    stddev_a: f64,
    stddev_b: f64,
    width_px: f64,
    height_px: f64,
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::ffi::{c_char, c_void, CString};
    use std::time::Duration;
    use tokio::sync::oneshot;

    type Done = extern "C" fn(*mut c_void, *const f64, i32);

    unsafe extern "C" {
        fn ah_collapse_animate(
            webview: *mut c_void,
            window: *mut c_void,
            json: *const c_char,
            done: Done,
            user: *mut c_void,
        );
        fn ah_collapse_prepare(
            webview: *mut c_void,
            left: f64,
            width: f64,
            height: f64,
            visible_left: f64,
        );
        fn ah_collapse_resume(
            webview: *mut c_void,
            window: *mut c_void,
            left: f64,
            visible_left: f64,
            min_width: f64,
            min_height: f64,
        );
        fn ah_collapse_front(window: *mut c_void);
        fn ah_collapse_activate_other() -> i32;
        fn ah_collapse_forbid_activation();
        fn ah_collapse_info(webview: *mut c_void, window: *mut c_void, out: *mut f64);
        fn ah_collapse_capture(
            window: *mut c_void,
            label: *const c_char,
            path: *const c_char,
        ) -> i32;
        fn ah_collapse_diff(
            label_a: *const c_char,
            label_b: *const c_char,
            x: f64,
            top: f64,
            width: f64,
            height: f64,
            out: *mut f64,
        ) -> i32;
        fn ah_collapse_burst_start(
            window: *mut c_void,
            directory: *const c_char,
            max_frames: i32,
            pause_microseconds: i32,
            write_all: i32,
        ) -> i32;
        fn ah_collapse_burst_stop() -> i32;
        fn ah_collapse_burst_tears() -> i32;
        fn ah_collapse_control(
            window: *mut c_void,
            mode: *const c_char,
            cycles: i32,
            rest_ms: i32,
            from_width: f64,
            from_height: f64,
            to_width: f64,
            to_height: f64,
            done: extern "C" fn(*mut c_void),
            user: *mut c_void,
        );
        fn ah_collapse_backdrop(
            window: *mut c_void,
            enable: i32,
            x: f64,
            y: f64,
            width: f64,
            height: f64,
        );
    }

    extern "C" fn finished(user: *mut c_void, values: *const f64, count: i32) {
        // Ownership of the boxed sender is transferred back exactly once per animation.
        let sender = unsafe { Box::from_raw(user as *mut oneshot::Sender<Vec<f64>>) };
        let report = if values.is_null() || count <= 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(values, count as usize) }.to_vec()
        };
        let _ = sender.send(report);
    }

    extern "C" fn control_finished(user: *mut c_void) {
        let sender = unsafe { Box::from_raw(user as *mut oneshot::Sender<()>) };
        let _ = sender.send(());
    }

    /// Run reference animations on a plain window (see `ah_collapse_control`); resolves when they are done.
    pub async fn control(
        window: &WebviewWindow,
        mode: String,
        cycles: i32,
        rest_ms: i32,
        sizes: [f64; 4],
    ) -> Result<(), String> {
        let mode = CString::new(mode).map_err(|e| e.to_string())?;
        let (tx, rx) = oneshot::channel::<()>();
        let user = Box::into_raw(Box::new(tx)) as usize;
        let scheduled = window.with_webview(move |platform| unsafe {
            ah_collapse_control(
                platform.ns_window(),
                mode.as_ptr(),
                cycles,
                rest_ms,
                sizes[0],
                sizes[1],
                sizes[2],
                sizes[3],
                control_finished,
                user as *mut c_void,
            );
        });
        if let Err(error) = scheduled {
            drop(unsafe { Box::from_raw(user as *mut oneshot::Sender<()>) });
            return Err(error.to_string());
        }
        let wait = (cycles as u64) * (2 * rest_ms as u64 + 1500) + 10_000;
        tokio::time::timeout(Duration::from_millis(wait), rx)
            .await
            .map_err(|_| "control animation timed out".to_string())?
            .map_err(|_| "control animation callback dropped".to_string())
    }

    pub async fn animate(
        window: &WebviewWindow,
        params: serde_json::Value,
    ) -> Result<Report, String> {
        let wait = params["durationMs"].as_f64().unwrap_or(0.0) + 5000.0;
        let json = CString::new(params.to_string()).map_err(|e| e.to_string())?;
        let (tx, rx) = oneshot::channel::<Vec<f64>>();
        let user = Box::into_raw(Box::new(tx)) as usize;
        let scheduled = window.with_webview(move |platform| unsafe {
            ah_collapse_animate(
                platform.inner(),
                platform.ns_window(),
                json.as_ptr(),
                finished,
                user as *mut c_void,
            );
        });
        if let Err(error) = scheduled {
            // The closure never ran, so the sender is still ours to drop.
            drop(unsafe { Box::from_raw(user as *mut oneshot::Sender<Vec<f64>>) });
            return Err(error.to_string());
        }
        let values = tokio::time::timeout(Duration::from_millis(wait as u64), rx)
            .await
            .map_err(|_| "native animation timed out".to_string())?
            .map_err(|_| "native animation callback dropped".to_string())?;
        Ok(Report::from_values(&values))
    }

    pub async fn prepare(
        window: &WebviewWindow,
        left: f64,
        width: f64,
        height: f64,
        visible_left: f64,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                ah_collapse_prepare(platform.inner(), left, width, height, visible_left);
                let _ = tx.send(());
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "prepare callback dropped".to_string())
    }

    pub async fn resume(
        window: &WebviewWindow,
        left: f64,
        visible_left: f64,
        min_w: f64,
        min_h: f64,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                ah_collapse_resume(
                    platform.inner(),
                    platform.ns_window(),
                    left,
                    visible_left,
                    min_w,
                    min_h,
                );
                let _ = tx.send(());
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "resume callback dropped".to_string())
    }

    pub async fn front(window: &WebviewWindow) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                ah_collapse_front(platform.ns_window());
                let _ = tx.send(());
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "front callback dropped".to_string())
    }

    /// Silent mode: this process never activates itself (see `ah_collapse_forbid_activation`). Call before the event loop starts.
    pub fn forbid_activation() {
        unsafe { ah_collapse_forbid_activation() }
    }

    /// Test hook: hand the activation to another app (Finder), as a click elsewhere would.
    pub async fn deactivate(window: &WebviewWindow) -> Result<bool, String> {
        let (tx, rx) = oneshot::channel::<i32>();
        window
            .with_webview(move |_| unsafe {
                let _ = tx.send(ah_collapse_activate_other());
            })
            .map_err(|e| e.to_string())?;
        Ok(rx
            .await
            .map_err(|_| "deactivate callback dropped".to_string())?
            == 1)
    }

    /// Show or remove the plain-colour backdrop right behind the answer window (`region` = x, y, w, h in points,
    /// top-left origin on the main display).
    pub async fn backdrop(
        window: &WebviewWindow,
        enable: bool,
        region: [f64; 4],
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                ah_collapse_backdrop(
                    platform.ns_window(),
                    enable as i32,
                    region[0],
                    region[1],
                    region[2],
                    region[3],
                );
                let _ = tx.send(());
            })
            .map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "backdrop callback dropped".to_string())
    }

    /// Capture this process's own window as composited (no screen-recording permission needed).
    pub async fn capture(
        window: &WebviewWindow,
        label: String,
        path: Option<String>,
    ) -> Result<(), String> {
        let label = CString::new(label).map_err(|e| e.to_string())?;
        let path = path
            .map(|p| CString::new(p).map_err(|e| e.to_string()))
            .transpose()?;
        let (tx, rx) = oneshot::channel::<i32>();
        window
            .with_webview(move |platform| unsafe {
                let code = ah_collapse_capture(
                    platform.ns_window(),
                    label.as_ptr(),
                    path.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                );
                let _ = tx.send(code);
            })
            .map_err(|e| e.to_string())?;
        match rx
            .await
            .map_err(|_| "capture callback dropped".to_string())?
        {
            0 => Ok(()),
            code => Err(format!("window capture failed ({code})")),
        }
    }

    /// Start capturing the window continuously into `directory` (see `ah_collapse_burst_start`).
    pub async fn burst_start(
        window: &WebviewWindow,
        directory: String,
        max_frames: i32,
        pause_microseconds: i32,
        write_all: bool,
    ) -> Result<(), String> {
        let directory = CString::new(directory).map_err(|e| e.to_string())?;
        let (tx, rx) = oneshot::channel::<i32>();
        window
            .with_webview(move |platform| unsafe {
                let code = ah_collapse_burst_start(
                    platform.ns_window(),
                    directory.as_ptr(),
                    max_frames,
                    pause_microseconds,
                    write_all as i32,
                );
                let _ = tx.send(code);
            })
            .map_err(|e| e.to_string())?;
        match rx.await.map_err(|_| "burst callback dropped".to_string())? {
            0 => Ok(()),
            code => Err(format!("burst capture failed to start ({code})")),
        }
    }

    /// Stop the running burst; returns `[frames, torn frames]`.
    pub async fn burst_stop() -> Result<Vec<i32>, String> {
        tokio::task::spawn_blocking(|| unsafe {
            let frames = ah_collapse_burst_stop();
            vec![frames, ah_collapse_burst_tears()]
        })
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn diff(
        window: &WebviewWindow,
        a: String,
        b: String,
        region: [f64; 4],
    ) -> Result<Diff, String> {
        let a = CString::new(a).map_err(|e| e.to_string())?;
        let b = CString::new(b).map_err(|e| e.to_string())?;
        let (tx, rx) = oneshot::channel::<(i32, Vec<f64>)>();
        window
            .with_webview(move |_| unsafe {
                let mut out = [0.0f64; 8];
                let code = ah_collapse_diff(
                    a.as_ptr(),
                    b.as_ptr(),
                    region[0],
                    region[1],
                    region[2],
                    region[3],
                    out.as_mut_ptr(),
                );
                let _ = tx.send((code, out.to_vec()));
            })
            .map_err(|e| e.to_string())?;
        let (code, v) = rx.await.map_err(|_| "diff callback dropped".to_string())?;
        if code != 0 {
            return Err(format!("capture diff failed ({code})"));
        }
        Ok(Diff {
            mean_abs: v[0],
            changed_fraction: v[1],
            stddev_a: v[2],
            stddev_b: v[3],
            width_px: v[4],
            height_px: v[5],
        })
    }

    pub async fn info(window: &WebviewWindow) -> Result<Info, String> {
        let (tx, rx) = oneshot::channel::<Vec<f64>>();
        window
            .with_webview(move |platform| unsafe {
                let mut out = [0.0f64; 24];
                ah_collapse_info(platform.inner(), platform.ns_window(), out.as_mut_ptr());
                let _ = tx.send(out.to_vec());
            })
            .map_err(|e| e.to_string())?;
        let v = rx.await.map_err(|_| "info callback dropped".to_string())?;
        Ok(Info {
            x: v[0],
            top: v[1],
            width: v[2],
            height: v[3],
            scale: v[4],
            screen_x: v[5],
            screen_top: v[6],
            screen_w: v[7],
            screen_h: v[8],
            level: v[9] as i64,
            always_on_top: v[9] as i64 > 0,
            min_w: v[10],
            min_h: v[11],
            lights_hidden: v[12] > 0.5,
            resizable: v[13] > 0.5,
            canvas_installed: v[14] > 0.5,
            canvas_responsive: v[15] > 0.5,
            canvas_x: v[16],
            view_w: v[17],
            view_h: v[18],
            visible: v[19] > 0.5,
            key: v[20] > 0.5,
            lights_alpha: v[21],
            has_shadow: v[22] > 0.5,
        })
    }
}

// Windows / Linux keep the same commands so the example builds everywhere, but only apply the
// final geometry instantly (spec C6: no animation outside macOS).
#[cfg(not(target_os = "macos"))]
mod mac {
    use super::*;

    pub async fn animate(
        window: &WebviewWindow,
        params: serde_json::Value,
    ) -> Result<Report, String> {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let position = window
            .outer_position()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(scale);
        let size = window
            .outer_size()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(scale);
        let width = params["toWidth"].as_f64().unwrap_or(size.width);
        let height = params["toHeight"].as_f64().unwrap_or(size.height);
        let x = position.x + params["toDX"].as_f64().unwrap_or(0.0);
        window
            .set_size(tauri::LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(tauri::LogicalPosition::new(x, position.y))
            .map_err(|e| e.to_string())?;
        Ok(Report {
            frames: 1.0,
            final_x: x,
            final_w: width,
            final_h: height,
            ..Default::default()
        })
    }
    pub async fn prepare(_: &WebviewWindow, _: f64, _: f64, _: f64, _: f64) -> Result<(), String> {
        Ok(())
    }
    pub async fn resume(_: &WebviewWindow, _: f64, _: f64, _: f64, _: f64) -> Result<(), String> {
        Ok(())
    }
    pub async fn front(window: &WebviewWindow) -> Result<(), String> {
        window.show().map_err(|e| e.to_string())
    }
    pub async fn backdrop(_: &WebviewWindow, _: bool, _: [f64; 4]) -> Result<(), String> {
        Err("the backdrop is only available on macOS".into())
    }
    pub fn forbid_activation() {}
    pub async fn deactivate(_: &WebviewWindow) -> Result<bool, String> {
        Err("handing the activation to another app is only available on macOS".into())
    }
    pub async fn control(
        _: &WebviewWindow,
        _: String,
        _: i32,
        _: i32,
        _: [f64; 4],
    ) -> Result<(), String> {
        Err("control animations are only available on macOS".into())
    }
    pub async fn capture(_: &WebviewWindow, _: String, _: Option<String>) -> Result<(), String> {
        Err("window capture is only available on macOS".into())
    }
    pub async fn burst_start(
        _: &WebviewWindow,
        _: String,
        _: i32,
        _: i32,
        _: bool,
    ) -> Result<(), String> {
        Err("window capture is only available on macOS".into())
    }
    pub async fn burst_stop() -> Result<Vec<i32>, String> {
        Err("window capture is only available on macOS".into())
    }
    pub async fn diff(
        _: &WebviewWindow,
        _: String,
        _: String,
        _: [f64; 4],
    ) -> Result<Diff, String> {
        Err("window capture is only available on macOS".into())
    }
    pub async fn info(window: &WebviewWindow) -> Result<Info, String> {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let position = window
            .outer_position()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(scale);
        let size = window
            .outer_size()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(scale);
        Ok(Info {
            x: position.x,
            top: position.y,
            width: size.width,
            height: size.height,
            scale,
            ..Default::default()
        })
    }
}

#[tauri::command]
async fn demo_info(window: WebviewWindow) -> Result<Info, String> {
    mac::info(&window).await
}

#[tauri::command]
async fn demo_prepare(
    window: WebviewWindow,
    left: f64,
    width: f64,
    height: f64,
    visible_left: f64,
) -> Result<(), String> {
    mac::prepare(&window, left, width, height, visible_left).await
}

#[tauri::command]
async fn demo_resume(
    window: WebviewWindow,
    left: f64,
    visible_left: f64,
    min_w: f64,
    min_h: f64,
) -> Result<(), String> {
    mac::resume(&window, left, visible_left, min_w, min_h).await
}

#[tauri::command]
async fn demo_animate(window: WebviewWindow, params: serde_json::Value) -> Result<Report, String> {
    mac::animate(&window, params).await
}

#[tauri::command]
async fn demo_front(window: WebviewWindow) -> Result<(), String> {
    mac::front(&window).await
}

/// Scripted tests: another app becomes active, like a click outside AskHuman (resolves to whether macOS accepted it).
#[tauri::command]
async fn demo_deactivate(window: WebviewWindow) -> Result<bool, String> {
    mac::deactivate(&window).await
}

/// Tell the pages whether AskHuman (any demo window) holds the focus. The answer window shrinks a peek at only
/// "稍后答" requests back to the small window when the user leaves (C23). Focus can move between two windows of this
/// app (the controller): only report the app as inactive when nobody holds the focus after a short grace period.
fn report_focus(app: &tauri::AppHandle, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::Focused(true) => {
            let _ = app.emit("demo-app-active", ());
        }
        tauri::WindowEvent::Focused(false) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let held = app
                    .webview_windows()
                    .values()
                    .any(|window| window.is_focused().unwrap_or(false));
                if !held {
                    let _ = app.emit("demo-app-inactive", ());
                }
            });
        }
        _ => {}
    }
}

/// Where `demo_capture` writes PNG files (`ASKHUMAN_DEMO_SHOTS` overrides the default).
fn shots_dir() -> std::path::PathBuf {
    std::env::var_os("ASKHUMAN_DEMO_SHOTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../.askhuman-dev/popup-collapse-shots")
        })
}

/// Store a capture of the answer window under `label`. With `path` (a plain file name) the capture is
/// also written as a PNG into the shots directory.
#[tauri::command]
async fn demo_capture(
    window: WebviewWindow,
    label: String,
    path: Option<String>,
) -> Result<(), String> {
    let path = match path {
        Some(name) => {
            let name = std::path::Path::new(&name)
                .file_name()
                .ok_or("invalid capture file name")?
                .to_owned();
            let dir = shots_dir();
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            Some(dir.join(name).to_string_lossy().into_owned())
        }
        None => None,
    };
    mac::capture(&window, label, path).await
}

/// Compare one screen-space region (x, top, width, height; AppKit coordinates) of two captures.
#[tauri::command]
async fn demo_diff(
    window: WebviewWindow,
    a: String,
    b: String,
    region: [f64; 4],
) -> Result<Diff, String> {
    mac::diff(&window, a, b, region).await
}

/// Run reference animations on a plain window in place of the answer window (frame-hitch isolation).
#[tauri::command]
async fn demo_control(
    window: WebviewWindow,
    mode: String,
    cycles: i32,
    rest_ms: i32,
    sizes: [f64; 4],
) -> Result<(), String> {
    mac::control(&window, mode, cycles, rest_ms, sizes).await
}

/// Put a plain-colour window behind the answer window (or remove it) for outside screen recordings.
#[tauri::command]
async fn demo_backdrop(
    window: WebviewWindow,
    enable: bool,
    region: [f64; 4],
) -> Result<(), String> {
    mac::backdrop(&window, enable, region).await
}

/// Capture the answer window continuously into `<shots>/<name>` until `demo_burst_stop`.
#[tauri::command]
async fn demo_burst_start(
    window: WebviewWindow,
    name: String,
    max_frames: i32,
    pause_microseconds: i32,
    write_all: bool,
) -> Result<(), String> {
    let name = std::path::Path::new(&name)
        .file_name()
        .ok_or("invalid burst name")?
        .to_owned();
    let dir = shots_dir().join(name);
    // A previous run of the same name must not leave stale frames behind.
    let _ = std::fs::remove_dir_all(&dir);
    mac::burst_start(
        &window,
        dir.to_string_lossy().into_owned(),
        max_frames,
        pause_microseconds,
        write_all,
    )
    .await
}

/// Stop the running burst capture; returns `[frames captured, torn frames]`.
#[tauri::command]
async fn demo_burst_stop() -> Result<Vec<i32>, String> {
    mac::burst_stop().await
}

#[tauri::command]
fn demo_pin(window: WebviewWindow, on: bool) -> Result<(), String> {
    window.set_always_on_top(on).map_err(|e| e.to_string())
}

#[tauri::command]
fn demo_ready(window: WebviewWindow, state: tauri::State<'_, DemoState>) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    if state.silent {
        return Ok(());
    }
    window.set_focus().map_err(|e| e.to_string())
}

/// The mock round is over (queue cleared): hide like the product host does; `demo_reset` starts the next one.
#[tauri::command]
fn demo_end_round(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
fn demo_quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn demo_record(
    app: tauri::AppHandle,
    state: tauri::State<'_, DemoState>,
    value: serde_json::Value,
    finished: bool,
) -> Result<(), String> {
    let path = std::env::var("ASKHUMAN_DEMO_LOG").unwrap_or_else(|_| {
        format!(
            "{}/../.askhuman-dev/popup-collapse-manual.jsonl",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{value}").map_err(|e| e.to_string())?;
    if finished && state.automatic {
        let code = if value["pass"].as_bool().unwrap_or(false) {
            0
        } else {
            1
        };
        TEST_EXIT_CODE.store(code, Ordering::SeqCst);
        app.exit(code);
    }
    Ok(())
}

fn open_popup(
    app: &tauri::AppHandle,
    effect: &str,
    scenario: Option<&str>,
    silent: bool,
) -> tauri::Result<()> {
    let mut url = format!("prototype/popup-collapse.html?surface=popup&effect={effect}");
    if silent {
        url.push_str("&silent=1");
    }
    if let Some(mode) = scenario {
        url.push_str(&format!("&auto={mode}"));
    }
    // Extra query parameters for scripted scenarios (for example `cycles=40`).
    if let Ok(extra) = std::env::var("ASKHUMAN_DEMO_QUERY") {
        url.push('&');
        url.push_str(&extra);
    }
    let builder = WebviewWindowBuilder::new(app, POPUP, WebviewUrl::App(url.into()))
        .title("AskHuman · 折叠态原型")
        .inner_size(560.0, 652.0)
        .min_inner_size(420.0, 480.0)
        .position(470.0, 90.0)
        .always_on_top(true)
        .visible(false);
    #[cfg(target_os = "macos")]
    let builder = {
        let builder = builder
            .transparent(true)
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
        match effect {
            "solid" => builder.background_color(tauri::window::Color(0xf0, 0xf0, 0xf2, 0xff)),
            "glass" => builder,
            _ => builder.effects(
                EffectsBuilder::new()
                    .effect(Effect::UnderWindowBackground)
                    .state(EffectState::FollowsWindowActiveState)
                    .build(),
            ),
        }
    };
    let window = builder.build()?;
    #[cfg(target_os = "macos")]
    if effect == "glass" {
        use tauri_plugin_liquid_glass::{LiquidGlassConfig, LiquidGlassExt};
        let _ = window
            .liquid_glass()
            .set_effect(&window, LiquidGlassConfig::default());
    }
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            // The product opens an in-window close confirmation here; the prototype only reports it.
            api.prevent_close();
            let _ = handle.emit("demo-action", "close");
        }
        report_focus(handle.app_handle(), event);
    });
    Ok(())
}

#[tauri::command]
async fn demo_reset(
    app: tauri::AppHandle,
    state: tauri::State<'_, DemoState>,
    effect: String,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(POPUP) {
        window.destroy().map_err(|e| e.to_string())?;
        // Tauri removes the label on the event loop after destruction; recreating in the same
        // synchronous command can fail with a duplicate label.
        for _ in 0..125 {
            if app.get_webview_window(POPUP).is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        }
        if app.get_webview_window(POPUP).is_some() {
            return Err("旧作答窗口未完成关闭，请重启原型".into());
        }
    }
    open_popup(&app, &effect, state.scenario.as_deref(), state.silent).map_err(|e| e.to_string())
}

fn main() {
    // ASKHUMAN_DEMO_AUTOTEST=1: scripted geometry scenario in the answer window.
    // ASKHUMAN_DEMO_AUTOTEST=controls: the controller window drives the answer window through its
    // real event channel and verifies the replies.
    let mode = std::env::var("ASKHUMAN_DEMO_AUTOTEST").ok();
    let automatic = mode.is_some();
    let controls_test = mode.as_deref() == Some("controls");
    // ASKHUMAN_DEMO_AUTOTEST=burst: record composited frames during transitions (no pass / fail).
    // ASKHUMAN_DEMO_AUTOTEST=video: transitions on a plain backdrop while an outside screen recording runs.
    let scenario = if automatic && !controls_test {
        Some(
            match mode.as_deref() {
                Some("burst") => "burst",
                Some("video") => "video",
                _ => "1",
            }
            .to_string(),
        )
    } else {
        None
    };
    let effect = std::env::var("ASKHUMAN_DEMO_EFFECT").unwrap_or_else(|_| "blur".into());
    // The geometry checks and the controller self-test run silently unless `--focus` asks for the real app switch.
    // The frame-pacing runs (burst / video) measure the window in its normal active state and stay as they were.
    let silent = matches!(mode.as_deref(), Some("1") | Some("controls"))
        && std::env::var_os("ASKHUMAN_DEMO_FOCUS").is_none();
    if silent {
        mac::forbid_activation();
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .manage(DemoState {
            automatic,
            scenario: scenario.clone(),
            silent,
        })
        .invoke_handler(tauri::generate_handler![
            demo_info,
            demo_prepare,
            demo_resume,
            demo_animate,
            demo_front,
            demo_deactivate,
            demo_capture,
            demo_diff,
            demo_burst_start,
            demo_burst_stop,
            demo_backdrop,
            demo_control,
            demo_pin,
            demo_ready,
            demo_end_round,
            demo_quit,
            demo_record,
            demo_reset
        ])
        .setup(move |app| {
            if !automatic || controls_test {
                let mut url =
                    format!("prototype/popup-collapse.html?surface=controls&effect={effect}");
                if controls_test {
                    url.push_str("&selftest=1");
                }
                // A silent run creates the controller hidden and unfocused and shows it once the loop runs (like the
                // answer window): a window that is key at launch makes AppKit activate the app.
                let controls =
                    WebviewWindowBuilder::new(app, CONTROLS, WebviewUrl::App(url.into()))
                        .title("折叠态 · Demo 控制器")
                        .inner_size(420.0, 820.0)
                        .position(30.0, 40.0)
                        .visible(!silent)
                        .focused(!silent)
                        .build()?;
                if silent {
                    let later = controls.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        let _ = later.show();
                    });
                }
                let handle = app.handle().clone();
                controls.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        handle.exit(0);
                    }
                    report_focus(&handle, event);
                });
            }
            open_popup(app.handle(), &effect, scenario.as_deref(), silent)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("native collapse demo failed");
    // The macOS event loop can return normally even after app.exit(1); keep the test status.
    if automatic {
        std::process::exit(TEST_EXIT_CODE.load(Ordering::SeqCst));
    }
}
