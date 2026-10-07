//! Exercise the production native appearance path without daemon or IM traffic.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../src/app/popup_transition.rs"]
mod popup_transition;

#[cfg(target_os = "macos")]
async fn state(window: &tauri::WebviewWindow) -> Result<serde_json::Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .with_webview(move |platform| unsafe {
            use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject};
            let native = platform.ns_window() as *mut AnyObject;
            let behavior: isize = msg_send![native, animationBehavior];
            let key: bool = msg_send![native, isKeyWindow];
            let visible: bool = msg_send![native, isVisible];
            let frame: objc2_foundation::NSRect = msg_send![native, frame];
            let workspace: *mut AnyObject =
                msg_send![AnyClass::get(c"NSWorkspace").unwrap(), sharedWorkspace];
            let front: *mut AnyObject = msg_send![workspace, frontmostApplication];
            let pid: i32 = msg_send![front, processIdentifier];
            let _ = tx.send(serde_json::json!({ "behavior": behavior, "key": key, "visible": visible,
                "frame": [frame.origin.x, frame.origin.y, frame.size.width, frame.size.height], "frontPid": pid }));
        })
        .map_err(|error| error.to_string())?;
    rx.await.map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn check_focus(before: &serde_json::Value, after: &serde_json::Value) -> Result<(), String> {
    if before["frontPid"] != after["frontPid"] || after["key"] != false {
        return Err(format!(
            "appearance moved keyboard focus: {before} -> {after}"
        ));
    }
    if before["frame"] != after["frame"] {
        return Err(format!(
            "appearance changed native frame: {before} -> {after}"
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn check(window: &tauri::WebviewWindow) -> Result<(), String> {
    use std::time::Duration;
    // Wry asks NSApplication to activate while creating its WebView. Keep that
    // startup side effect out of the native ordering comparison.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let baseline = state(window).await?;
    let control = serde_json::to_value(popup_transition::front(window).await?).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let control_after = state(window).await?;
    println!(
        "{}",
        serde_json::json!({"control": control, "before": baseline, "after": control_after, "ownPid": std::process::id()})
    );
    check_focus(&baseline, &control_after)?;
    window.hide().map_err(|error| error.to_string())?;
    tokio::time::sleep(Duration::from_millis(450)).await;
    for (round, behavior) in [5, 2, 3, 5].into_iter().enumerate() {
        let before = state(window).await?;
        if before["frontPid"] == std::process::id() {
            return Err("start this probe while another application has focus".into());
        }
        let shown = serde_json::to_value(popup_transition::appear(window, behavior).await?)
            .map_err(|error| error.to_string())?;
        println!(
            "{}",
            serde_json::json!({"shown": shown, "ownPid": std::process::id()})
        );
        if shown["visibleBefore"] != false || shown["appearBehaviorApplied"] != behavior {
            return Err(format!("hidden round did not apply behavior: {shown}"));
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        let after = state(window).await?;
        check_focus(&before, &after)?;
        if after["behavior"] != behavior || after["visible"] != true {
            return Err(format!(
                "native appearance setting was not retained: {after}"
            ));
        }
        let raised = serde_json::to_value(popup_transition::front(window).await?)
            .map_err(|error| error.to_string())?;
        if raised["visibleBefore"] != true || !raised["appearBehaviorApplied"].is_null() {
            return Err(format!(
                "visible arrival reapplied opening animation: {raised}"
            ));
        }
        check_focus(&after, &state(window).await?)?;
        println!(
            "{}",
            serde_json::json!({ "round": round + 1, "behavior": behavior,
            "focusStable": true, "frameStable": true, "visibleRaiseNoAppearance": true })
        );
        window.hide().map_err(|error| error.to_string())?;
        tokio::time::sleep(Duration::from_millis(450)).await;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() {
    use std::sync::{
        atomic::{AtomicI32, Ordering},
        Arc,
    };
    use tauri::Manager;
    let result = Arc::new(AtomicI32::new(1));
    let app = tauri::Builder::default()
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Prohibited);
            tauri::WebviewWindowBuilder::new(
                app,
                "appearance-regression",
                tauri::WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .title("AskHuman native appearance regression")
            .inner_size(560.0, 620.0)
            .visible(false)
            .focused(false)
            .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("native appearance regression runtime failed");
    let result_for_run = result.clone();
    app.run(move |app, event| {
        if matches!(event, tauri::RunEvent::Ready) {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let window = app.get_webview_window("appearance-regression").unwrap();
            let app = app.clone();
            let result = result_for_run.clone();
            tauri::async_runtime::spawn(async move {
                match check(&window).await {
                    Ok(()) => {
                        result.store(0, Ordering::SeqCst);
                        app.exit(0);
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        app.exit(1);
                    }
                }
            });
        }
    });
    std::process::exit(result.load(Ordering::SeqCst));
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("native appearance regression requires macOS");
}
