//! Exercise popup navigation protection in the real WebView, without daemon or IM traffic.
//! Run: cargo run --manifest-path src-tauri/Cargo.toml --example popup_navigation_regression --features custom-protocol
#[path = "../src/app/popup_navigation.rs"]
mod popup_navigation;

use std::{borrow::Cow, sync::Mutex};
use tauri::{
    utils::assets::{AssetKey, AssetsIter, CspHash},
    Assets, Manager, Wry,
};

struct Page;
impl Assets<Wry> for Page {
    fn get(&self, _key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        // Match the production SPA fallback: a leaked local path serves the app again.
        Some(Cow::Borrowed(include_bytes!(
            "popup_navigation_regression.html"
        )))
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }
    fn csp_hashes(&self, _path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}

#[derive(Default)]
struct Reports(Mutex<Vec<serde_json::Value>>);

#[tauri::command]
fn report_results(app: tauri::AppHandle, report: serde_json::Value) {
    println!("{report}");
    if report["passed"] != true {
        std::process::exit(1);
    }
    let reports = app.state::<Reports>();
    let mut reports = reports.0.lock().unwrap();
    reports.push(report);
    if reports.len() == 2 {
        println!("PASS: independent and shared popup native navigation regression");
        std::process::exit(0);
    }
}

fn main() {
    let mut context = tauri::generate_context!();
    context.config_mut().build.dev_url = None;
    context.set_assets(Box::new(Page));
    tauri::Builder::default()
        .manage(Reports::default())
        .invoke_handler(tauri::generate_handler![report_results])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Prohibited);
            for view in ["popup", "popup-inbox"] {
                let route = format!("index.html?view={view}&effect=solid");
                let document = popup_navigation::document_url(app.config(), &route)?;
                let disable_guard =
                    std::env::var_os("ASKHUMAN_NAVIGATION_NEGATIVE_CONTROL").is_some();
                tauri::WebviewWindowBuilder::new(app, view, tauri::WebviewUrl::App(route.into()))
                    .initialization_script(include_str!("../src/app/popup_navigation.js"))
                    .on_navigation(move |destination| {
                        disable_guard || popup_navigation::allows(&document, destination)
                    })
                    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                    .visible(false)
                    .focused(false)
                    .build()?;
            }
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(20));
                eprintln!(
                    "FAIL: navigation regression timed out (document may have been replaced)"
                );
                std::process::exit(2);
            });
            Ok(())
        })
        .run(context)
        .expect("native navigation regression failed");
    std::process::exit(2);
}
