//! Original-file actions for the Popup menu, isolated from other attachment entry points.
use tauri::AppHandle;
#[cfg(not(target_os = "macos"))]
use tauri::Emitter;
#[cfg(not(target_os = "macos"))]
use tauri::Manager;
#[cfg(not(target_os = "macos"))]
pub fn reveal(path: &str) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = Command::new("explorer.exe");
        c.arg(format!("/select,{path}"));
        c
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let uri = reqwest::Url::from_file_path(path).map_err(|_| "invalid attachment path")?;
        let items = serde_json::to_string(&vec![uri.as_str()]).map_err(|e| e.to_string())?;
        let mut c = Command::new("gdbus");
        c.args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItems",
        ]);
        c.arg(items).arg("");
        c
    };
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let status = command.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("file manager could not reveal attachment".into())
    }
}
#[cfg(not(target_os = "macos"))]
pub fn menu(window: &tauri::Window, request_id: &str, index: usize) -> Result<(), String> {
    use tauri::menu::{ContextMenu, Menu, MenuItem};
    let lang = crate::i18n::Lang::current();
    let request = super::popup_preview::request(window, request_id)?;
    let file = request
        .message
        .files
        .get(index)
        .ok_or("invalid attachment index")?;
    let definitions = [
        ("open", crate::i18n::tr(lang, "menu.open").to_owned()),
        (
            "preview",
            crate::i18n::tr(lang, "menu.quickLook").replace("{name}", &file.name),
        ),
        (
            "reveal",
            if matches!(lang, crate::i18n::Lang::Zh) {
                "在文件管理器中显示"
            } else if cfg!(target_os = "windows") {
                "Show in File Explorer"
            } else {
                "Show in file manager"
            }
            .to_owned(),
        ),
        ("copy", crate::i18n::tr(lang, "menu.copyPath").to_owned()),
    ];
    let menu = Menu::new(window).map_err(|e| e.to_string())?;
    for (action, text) in definitions {
        let item = MenuItem::with_id(
            window,
            format!("popup-preview:{request_id}:{index}:{action}"),
            text,
            true,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        menu.append(&item).map_err(|e| e.to_string())?;
    }
    // Retain the menu while its native context popup is open; a subsequent popup replaces it.
    let state = window.app_handle().state::<MenuOwner>();
    menu.popup(window.clone()).map_err(|e| e.to_string())?;
    *state.0.lock().unwrap() = Some(menu);
    Ok(())
}
#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct MenuOwner(std::sync::Mutex<Option<tauri::menu::Menu<tauri::Wry>>>);
#[cfg(target_os = "macos")]
pub fn handle_menu(_: &AppHandle, _: &str) {}
#[cfg(not(target_os = "macos"))]
pub fn handle_menu(app: &AppHandle, id: &str) {
    let Some(id) = id.strip_prefix("popup-preview:") else {
        return;
    };
    let mut parts = id.rsplitn(3, ':');
    let (Some(action), Some(index), Some(request_id)) = (parts.next(), parts.next(), parts.next())
    else {
        return;
    };
    let Ok(index) = index.parse::<usize>() else {
        return;
    };
    let Some(window) = app.get_webview_window("popup").map(|w| w.as_ref().window()) else {
        return;
    };
    let Ok(request) = super::popup_preview::request(&window, request_id) else {
        return;
    };
    let Some(file) = request.message.files.get(index) else {
        return;
    };
    let target = serde_json::json!({ "requestId": request_id, "index": index });
    match action {
        "preview" => {
            let _ = window.emit("popup-preview-show", target);
        }
        "copy" => {
            let _ = window.emit("popup-preview-copy-path", target);
        }
        "open" => {
            let _ = crate::commands::open_path(file.path.clone());
        }
        "reveal" => {
            let path = file.path.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if reveal(&path).is_err() {
                    let _ = window.emit("popup-preview-action-failed", target);
                }
            });
        }
        _ => {}
    }
}
