//! Run the product pulse against a real WindowServer window, without daemon/IM traffic.
#[cfg(target_os = "macos")]
#[path = "../src/app/popup_pulse.rs"]
mod popup_pulse;
#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../src/app/popup_transition.rs"]
mod popup_transition;
#[cfg(target_os = "macos")]
mod dev_instance {
    pub fn is_dev_instance() -> bool {
        false
    }
}
#[cfg(target_os = "macos")]
mod paths {
    pub fn state_dir() -> std::path::PathBuf {
        std::env::temp_dir()
    }
}

#[cfg(target_os = "macos")]
async fn transform(window: &tauri::WebviewWindow) -> Result<[f64; 6], String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .with_webview(move |platform| unsafe {
            use objc2::{msg_send, runtime::AnyObject};
            #[repr(C)]
            #[derive(Default)]
            struct Transform {
                a: f64,
                b: f64,
                c: f64,
                d: f64,
                tx: f64,
                ty: f64,
            }
            let handle = libc::dlopen(
                c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
                libc::RTLD_LAZY,
            );
            let result = (|| {
                if handle.is_null() {
                    return Err("SkyLight unavailable".to_string());
                }
                let connection = libc::dlsym(handle, c"SLSMainConnectionID".as_ptr());
                let get = libc::dlsym(handle, c"SLSGetWindowTransform".as_ptr());
                if connection.is_null() || get.is_null() {
                    return Err("SkyLight symbols unavailable".into());
                }
                let connection = std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn() -> i32,
                >(connection);
                let get = std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(i32, u32, *mut Transform) -> i32,
                >(get);
                let native = platform.ns_window() as *mut AnyObject;
                let id: i64 = msg_send![native, windowNumber];
                let mut value = Transform::default();
                if get(connection(), id as u32, &mut value) != 0 {
                    return Err("transform readback failed".into());
                }
                Ok([value.a, value.b, value.c, value.d, value.tx, value.ty])
            })();
            if !handle.is_null() {
                libc::dlclose(handle);
            }
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

#[cfg(target_os = "macos")]
async fn check(window: &tauri::WebviewWindow) -> Result<(), String> {
    use std::time::Duration;
    popup_pulse::watch_interaction(window)?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let baseline = transform(window).await?;
    let pulse = popup_pulse::pulse(window).await?;
    if !pulse.frame_stable || !pulse.transform_restored || pulse.peak_scale < 1.02 {
        return Err("single pulse did not restore native geometry".into());
    }
    for delay in [30, 50, 80, 100, 140, 180, 220, 280, 360, 440] {
        let (_, successor) = tokio::join!(popup_pulse::pulse(window), async {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            popup_pulse::pulse(window).await
        });
        let successor = successor?;
        let actual = transform(window).await?;
        let drift = baseline
            .iter()
            .zip(actual)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        if !successor.transform_restored || !successor.frame_stable || drift > 0.001 {
            return Err(format!(
                "overlap at {delay}ms left a transform drift of {drift}: {actual:?}"
            ));
        }
    }
    let (cancelled, _) = tokio::join!(popup_pulse::pulse(window), async {
        tokio::time::sleep(Duration::from_millis(80)).await;
        popup_pulse::cancel();
    });
    if !cancelled?.transform_restored || transform(window).await? != baseline {
        return Err("cancelled pulse did not restore its original transform".into());
    }
    println!(
        "{{\"single\":true,\"overlapDelaysPassed\":10,\"cancelled\":true,\"transformDrift\":0}}"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "pulse-regression",
                tauri::WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .title("AskHuman pulse regression")
            .inner_size(560.0, 620.0)
            .focused(false)
            .build()?;
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match check(&window).await {
                    Ok(()) => app.exit(0),
                    Err(error) => {
                        eprintln!("{error}");
                        app.exit(1);
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("native regression runtime failed");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("WindowServer regression requires macOS");
}
