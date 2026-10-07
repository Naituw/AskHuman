//! Verify production Windows presentation against actual HWND visibility and focus.
//! Run via scripts/popup-visibility-regression.ps1 to embed the example's Windows manifest.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
#[path = "../src/app/popup_transition.rs"]
mod popup_transition;

#[cfg(target_os = "windows")]
#[derive(Debug, PartialEq)]
struct State {
    visible: bool,
    minimized: bool,
    foreground: isize,
    frame: [i32; 4],
}

#[cfg(target_os = "windows")]
fn state(window: &tauri::WebviewWindow) -> Result<State, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowRect, IsIconic, IsWindowVisible,
    };
    let hwnd = window.hwnd().map_err(|error| error.to_string())?.0;
    let mut rect = windows_sys::Win32::Foundation::RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(State {
            visible: IsWindowVisible(hwnd) != 0,
            minimized: IsIconic(hwnd) != 0,
            foreground: GetForegroundWindow() as isize,
            frame: [rect.left, rect.top, rect.right, rect.bottom],
        })
    }
}

#[cfg(target_os = "windows")]
async fn settled() {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[cfg(target_os = "windows")]
async fn check(
    window: &tauri::WebviewWindow,
    witness: &tauri::WebviewWindow,
) -> Result<(), String> {
    let witness_hwnd = witness.hwnd().map_err(|error| error.to_string())?.0 as isize;
    settled().await;
    witness.set_focus().map_err(|error| error.to_string())?;
    settled().await;
    for always_on_top in [true, false] {
        window
            .set_always_on_top(always_on_top)
            .map_err(|error| error.to_string())?;
        settled().await;
        for round in 1..=3 {
            let before = state(window)?;
            if before.visible || before.foreground != witness_hwnd {
                return Err(format!("invalid hidden baseline: {before:?}"));
            }
            popup_transition::front(window).await?;
            settled().await;
            let shown = state(window)?;
            if !shown.visible
                || shown.minimized
                || shown.foreground != witness_hwnd
                || shown.frame != before.frame
            {
                return Err(format!("show changed visibility/focus/frame: {shown:?}"));
            }
            popup_transition::front(window).await?;
            settled().await;
            if state(window)? != shown {
                return Err("visible raise changed focus or frame".into());
            }
            window.hide().map_err(|error| error.to_string())?;
            settled().await;
            let hidden = state(window)?;
            if hidden.visible || hidden.foreground != witness_hwnd {
                return Err(format!("hide failed after native front: {hidden:?}"));
            }
            println!("round={round} alwaysOnTop={always_on_top} show=true hide=true focusStable=true frameStable=true");
        }
        popup_transition::front(window).await?;
        settled().await;
        window.minimize().map_err(|error| error.to_string())?;
        settled().await;
        if !state(window)?.minimized {
            return Err("minimize did not reach the native window".into());
        }
        witness.set_focus().map_err(|error| error.to_string())?;
        settled().await;
        popup_transition::front(window).await?;
        settled().await;
        let restored = state(window)?;
        if !restored.visible || restored.minimized || restored.foreground != witness_hwnd {
            return Err(format!("nonactivating restore failed: {restored:?}"));
        }
        window.hide().map_err(|error| error.to_string())?;
        settled().await;
        if state(window)?.visible {
            return Err("restored window did not hide".into());
        }
        println!("alwaysOnTop={always_on_top} minimizedRestore=true hide=true focusStable=true");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn main() {
    use tauri::Manager;
    let app = tauri::Builder::default()
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "focus-witness",
                tauri::WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .title("AskHuman regression focus witness")
            .inner_size(300.0, 200.0)
            .position(30.0, 30.0)
            .build()?;
            tauri::WebviewWindowBuilder::new(
                app,
                "visibility-regression",
                tauri::WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .title("AskHuman popup visibility regression")
            .inner_size(490.0, 560.0)
            .position(380.0, 30.0)
            .visible(false)
            .focused(false)
            .always_on_top(true)
            .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Windows visibility regression runtime failed");
    let code = app.run_return(move |app, event| {
        if matches!(event, tauri::RunEvent::Ready) {
            let window = app.get_webview_window("visibility-regression").unwrap();
            let witness = app.get_webview_window("focus-witness").unwrap();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match check(&window, &witness).await {
                    Ok(()) => app.exit(0),
                    Err(error) => {
                        eprintln!("{error}");
                        app.exit(1);
                    }
                }
            });
        }
    });
    std::process::exit(code);
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("native visibility regression requires Windows");
}
