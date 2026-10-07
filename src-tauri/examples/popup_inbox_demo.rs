//! Native, mock-only prototype. It does not connect to the product daemon or IM channels.
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicI32, Ordering},
        Mutex,
    },
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
mod inbox_demo_freeze;
mod inbox_demo_pulse;
static TEST_EXIT_CODE: AtomicI32 = AtomicI32::new(2);

#[derive(Clone, Copy)]
struct Anchor {
    x: f64,
    y: f64,
    main: f64,
    height: f64,
}
struct DemoState {
    anchor: Mutex<Option<Anchor>>,
    automatic: bool,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Layout {
    main: f64,
    sidebar: f64,
    preview: f64,
    width: f64,
    x: f64,
    y: f64,
    height: f64,
    limited: bool,
}

// Shrink the preview, then the sidebar, then the main region. Extremely narrow test areas
// retain the left/main/right order and divide remaining space without negative widths.
fn allocate(wanted: f64, sidebar: bool, preview: bool, available: f64) -> Layout {
    let gaps = if sidebar { 6.0 } else { 0.0 } + if preview { 6.0 } else { 0.0 };
    let mut main = wanted;
    let mut left = if sidebar { 240.0 } else { 0.0 };
    let mut right = if preview { 700.0 } else { 0.0 };
    let budget = (available - gaps).max(1.0);
    let mut deficit = (main + left + right - budget).max(0.0);
    for (value, minimum) in [
        (&mut right, if preview { 320.0 } else { 0.0 }),
        (&mut left, if sidebar { 180.0 } else { 0.0 }),
        (&mut main, 240.0),
    ] {
        let reduction = deficit.min((*value - minimum).max(0.0));
        *value -= reduction;
        deficit -= reduction;
    }
    if deficit > 0.0 {
        let ratio = budget / (main + left + right);
        main *= ratio;
        left *= ratio;
        right *= ratio;
    }
    Layout {
        main,
        sidebar: left,
        preview: right,
        width: main + left + right + gaps,
        x: 0.0,
        y: 0.0,
        height: 0.0,
        limited: main < wanted - 0.5,
    }
}

#[tauri::command]
fn demo_layout(
    window: WebviewWindow,
    state: tauri::State<'_, DemoState>,
    sidebar: bool,
    preview: bool,
    work_limit: Option<f64>,
    apply: bool,
) -> Result<Layout, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let position = window
        .outer_position()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    let size = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("no current monitor")?;
    let work = monitor.work_area();
    let monitor_scale = monitor.scale_factor();
    let wx = work.position.x as f64 / monitor_scale;
    let wy = work.position.y as f64 / monitor_scale;
    let ww = work.size.width as f64 / monitor_scale;
    let wh = work.size.height as f64 / monitor_scale;
    let available = work_limit.unwrap_or(ww).clamp(420.0, ww);
    let mut lock = state.anchor.lock().map_err(|e| e.to_string())?;
    let anchor = *lock.get_or_insert(Anchor {
        x: position.x,
        y: position.y,
        main: size.width,
        height: size.height,
    });
    let mut plan = allocate(anchor.main, sidebar, preview, available);
    // Keep the original main screen coordinate unless the entire work area forces a shift.
    plan.x = (anchor.x - plan.sidebar - if sidebar { 6.0 } else { 0.0 })
        .clamp(wx, wx + available - plan.width);
    plan.y = anchor.y.clamp(wy, wy + wh - anchor.height.min(wh));
    plan.height = anchor.height.min(wh);
    if apply {
        window
            .set_size(tauri::LogicalSize::new(plan.width, plan.height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(tauri::LogicalPosition::new(plan.x, plan.y))
            .map_err(|e| e.to_string())?;
    }
    Ok(plan)
}

#[tauri::command]
fn demo_finish(window: WebviewWindow) -> Result<(), String> {
    inbox_demo_freeze::release(&window);
    window.destroy().map_err(|e| e.to_string())
}

#[tauri::command]
async fn demo_transition_begin(window: WebviewWindow) -> Result<bool, String> {
    inbox_demo_freeze::begin(&window).await
}

#[tauri::command]
async fn demo_transition_apply(window: WebviewWindow, layout: Layout) -> Result<(), String> {
    inbox_demo_freeze::apply(&window, layout).await
}

#[tauri::command]
async fn demo_transition_end(window: WebviewWindow) -> Result<(), String> {
    inbox_demo_freeze::end(&window).await
}

#[tauri::command]
fn demo_transition_active() -> bool {
    inbox_demo_freeze::active()
}

#[tauri::command]
async fn demo_transition_probe(
    window: WebviewWindow,
) -> Result<Option<inbox_demo_freeze::Probe>, String> {
    inbox_demo_freeze::probe(&window).await
}

#[tauri::command]
async fn demo_notice_front(window: WebviewWindow) -> Result<inbox_demo_freeze::FrontProbe, String> {
    inbox_demo_freeze::front(&window).await
}

#[tauri::command]
async fn demo_notice_pulse(window: WebviewWindow) -> Result<inbox_demo_freeze::PulseProbe, String> {
    inbox_demo_pulse::pulse(&window).await
}

#[tauri::command]
async fn demo_test_minimize(
    window: WebviewWindow,
    state: tauri::State<'_, DemoState>,
) -> Result<(), String> {
    if !state.automatic {
        return Err("test-only operation".into());
    }
    window.minimize().map_err(|e| e.to_string())?;
    for _ in 0..125 {
        if window.is_minimized().map_err(|e| e.to_string())? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
    }
    Err("window did not minimize".into())
}

fn open_popup(app: &tauri::AppHandle, automatic: bool) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("inbox-demo") {
        inbox_demo_freeze::release(&window);
        window.destroy()?;
    }
    let url = if automatic {
        "prototype/popup-inbox.html?surface=popup&auto=1"
    } else {
        "prototype/popup-inbox.html?surface=popup"
    };
    let builder = WebviewWindowBuilder::new(app, "inbox-demo", WebviewUrl::App(url.into()))
        .title("AskHuman · 统一作答原型")
        .inner_size(560.0, 652.0)
        .min_inner_size(420.0, 480.0)
        .position(470.0, 100.0)
        .visible(false);
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    let window = builder.build()?;
    window.on_window_event({
        let window = window.clone();
        move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("demo-action", "close");
            }
        }
    });
    Ok(())
}

#[tauri::command]
async fn demo_reset(
    app: tauri::AppHandle,
    state: tauri::State<'_, DemoState>,
) -> Result<(), String> {
    *state.anchor.lock().map_err(|e| e.to_string())? = None;
    if let Some(window) = app.get_webview_window("inbox-demo") {
        inbox_demo_freeze::release(&window);
        window.destroy().map_err(|e| e.to_string())?;
        // Tauri removes the label on the event loop after destruction; recreating it in the
        // same synchronous command can fail with a duplicate label and leave no popup.
        for _ in 0..125 {
            if app.get_webview_window("inbox-demo").is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        }
        if app.get_webview_window("inbox-demo").is_some() {
            return Err("旧作答窗口未完成关闭，请重启原型".into());
        }
    }
    open_popup(&app, state.automatic).map_err(|e| e.to_string())
}

#[tauri::command]
fn demo_ready(window: WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
fn demo_record(
    app: tauri::AppHandle,
    state: tauri::State<'_, DemoState>,
    value: serde_json::Value,
    finished: bool,
) -> Result<(), String> {
    {
        let path = std::env::var("ASKHUMAN_DEMO_LOG").unwrap_or_else(|_| {
            format!(
                "{}/../.askhuman-dev/popup-inbox-manual.jsonl",
                env!("CARGO_MANIFEST_DIR")
            )
        });
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(file, "{value}").map_err(|e| e.to_string())?;
    }
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

fn main() {
    let automatic = std::env::var_os("ASKHUMAN_DEMO_AUTOTEST").is_some();
    tauri::Builder::default()
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .manage(DemoState {
            anchor: Mutex::new(None),
            automatic,
        })
        .invoke_handler(tauri::generate_handler![
            demo_layout,
            demo_finish,
            demo_reset,
            demo_ready,
            demo_record,
            demo_transition_begin,
            demo_transition_apply,
            demo_transition_end,
            demo_transition_active,
            demo_transition_probe,
            demo_notice_front,
            demo_notice_pulse,
            demo_test_minimize
        ])
        .setup(move |app| {
            if !automatic {
                WebviewWindowBuilder::new(
                    app,
                    "inbox-demo-controls",
                    WebviewUrl::App("prototype/popup-inbox.html?surface=controls".into()),
                )
                .title("统一作答 · Demo 控制器")
                .inner_size(400.0, 610.0)
                .position(40.0, 100.0)
                .build()?;
            }
            open_popup(app.handle(), automatic)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("native inbox demo failed");
    // The macOS event loop can return normally even after app.exit(1). Preserve test status
    // for the launcher rather than treating a failed native check as a successful run.
    if automatic {
        std::process::exit(TEST_EXIT_CODE.load(Ordering::SeqCst));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_shrinks_before_other_regions() {
        let p = allocate(560.0, true, true, 1200.0);
        assert_eq!((p.main, p.sidebar, p.preview), (560.0, 240.0, 388.0));
    }
    #[test]
    fn sidebar_shrinks_before_main() {
        let p = allocate(560.0, true, true, 1100.0);
        assert_eq!((p.main, p.sidebar, p.preview), (560.0, 208.0, 320.0));
        let p = allocate(560.0, true, true, 900.0);
        assert_eq!((p.main, p.sidebar, p.preview), (388.0, 180.0, 320.0));
    }
    #[test]
    fn extreme_widths_remain_bounded() {
        let p = allocate(560.0, true, true, 420.0);
        assert!((p.width - 420.0).abs() < 0.01);
        assert!(p.main > p.sidebar && p.preview > 0.0);
    }
}
