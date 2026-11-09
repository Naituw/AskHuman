//! End this instance before a native session quit can look like a popup host failure.

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use std::time::Duration;
    use tauri::AppHandle;

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static ENDING: AtomicBool = AtomicBool::new(false);
    static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" {
        fn ah_session_end_observe(ending: extern "C" fn());
    }

    fn stop_instance() {
        ENDING.store(true, Ordering::SeqCst);
        if STOP_REQUESTED.swap(true, Ordering::SeqCst) {
            return;
        }
        tauri::async_runtime::block_on(async {
            tokio::time::timeout(Duration::from_secs(2), crate::client::request_stop(true))
                .await
                .unwrap_or(false)
        });
    }

    extern "C" fn ending() {
        stop_instance();
        if let Some(app) = APP.get() {
            app.exit(0);
        }
    }

    pub fn observe(app: &AppHandle) {
        let _ = APP.set(app.clone());
        unsafe { ah_session_end_observe(ending) };
    }

    pub fn is_ending() -> bool {
        ENDING.load(Ordering::SeqCst)
    }
}

pub fn observe(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    platform::observe(app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

pub fn is_ending() -> bool {
    #[cfg(target_os = "macos")]
    return platform::is_ending();
    #[cfg(not(target_os = "macos"))]
    false
}
