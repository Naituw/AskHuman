//! Window-scoped image URLs. Only registered, bounded images can be served by this protocol.
use crate::image_resource::{self, ImageInfo};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
use tauri::http::{Method, Request, Response, StatusCode};

pub const SCHEME: &str = "askhuman-image";
const MAX_SCOPES: usize = 256;
const MAX_IMAGES_PER_SCOPE: usize = 128;
const MAX_SCOPE_PIXELS: u64 = image_resource::ANIMATION_PIXELS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: Option<SystemTime>,
}
impl Stamp {
    fn read(path: &Path) -> Result<Self, &'static str> {
        let metadata = std::fs::metadata(path).map_err(|_| "readFailed")?;
        if !metadata.is_file() {
            return Err("unsupported");
        }
        if metadata.len() > image_resource::IMAGE_BYTES as u64 {
            return Err("limit");
        }
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}
#[derive(Clone)]
struct Asset {
    path: PathBuf,
    stamp: Stamp,
    info: ImageInfo,
}
struct Scope {
    window: String,
    assets: HashMap<String, Asset>,
    pixels: u64,
}
#[derive(Clone)]
pub struct Registry {
    scopes: Arc<Mutex<HashMap<String, Scope>>>,
    pub workers: Arc<tokio::sync::Semaphore>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            scopes: Default::default(),
            workers: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub token: String,
    pub width: u32,
    pub height: u32,
}
impl Registry {
    pub fn create_scope(&self, window: &str) -> Result<String, &'static str> {
        let mut scopes = self.scopes.lock().unwrap();
        if scopes.len() >= MAX_SCOPES {
            return Err("limit");
        }
        let id = uuid::Uuid::new_v4().to_string();
        scopes.insert(
            id.clone(),
            Scope {
                window: window.to_owned(),
                assets: HashMap::new(),
                pixels: 0,
            },
        );
        Ok(id)
    }
    pub fn release_scope(&self, window: &str, scope: &str) {
        let mut scopes = self.scopes.lock().unwrap();
        if scopes.get(scope).is_some_and(|s| s.window == window) {
            scopes.remove(scope);
        }
    }
    pub fn release_window(&self, window: &str) {
        self.scopes
            .lock()
            .unwrap()
            .retain(|_, scope| scope.window != window);
    }
    fn active(&self, window: &str, id: &str) -> bool {
        self.scopes
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|s| s.window == window)
    }
    pub fn prepare(&self, window: &str, id: &str, path: &Path) -> Result<Prepared, &'static str> {
        if !self.active(window, id) {
            return Err("stale");
        }
        let path = path.canonicalize().map_err(|_| "readFailed")?;
        {
            let scopes = self.scopes.lock().unwrap();
            let scope = scopes
                .get(id)
                .filter(|s| s.window == window)
                .ok_or("stale")?;
            if let Some((token, asset)) = scope.assets.iter().find(|(_, a)| a.path == path) {
                return Ok(Prepared {
                    token: token.clone(),
                    width: asset.info.width,
                    height: asset.info.height,
                });
            }
        }
        let stamp = Stamp::read(&path)?;
        let bytes = image_resource::read(&path, image_resource::IMAGE_BYTES)?;
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let info = image_resource::inspect(&bytes, &extension)?;
        if Stamp::read(&path)? != stamp {
            return Err("readFailed");
        }
        // Scope cancellation can race disk inspection; never recreate a released scope.
        let mut scopes = self.scopes.lock().unwrap();
        let scope = scopes
            .get_mut(id)
            .filter(|s| s.window == window)
            .ok_or("stale")?;
        if scope.assets.len() >= MAX_IMAGES_PER_SCOPE
            || scope.pixels.saturating_add(info.display_pixels) > MAX_SCOPE_PIXELS
        {
            return Err("limit");
        }
        let token = uuid::Uuid::new_v4().to_string();
        scope.pixels += info.display_pixels;
        scope
            .assets
            .insert(token.clone(), Asset { path, stamp, info });
        Ok(Prepared {
            token,
            width: info.width,
            height: info.height,
        })
    }
    fn asset(&self, window: &str, token: &str) -> Option<Asset> {
        self.scopes
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.window == window)
            .find_map(|s| s.assets.get(token).cloned())
    }
    pub fn respond(&self, window: &str, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
        if !matches!(*request.method(), Method::GET | Method::HEAD) {
            return response(StatusCode::METHOD_NOT_ALLOWED, "text/plain", Vec::new(), 0);
        }
        let token = request.uri().path().strip_prefix('/').unwrap_or("");
        let Some(asset) = self
            .asset(window, token)
            .filter(|_| request.uri().query().is_none())
        else {
            return response(StatusCode::NOT_FOUND, "text/plain", Vec::new(), 0);
        };
        let content = || -> Result<Vec<u8>, &'static str> {
            if Stamp::read(&asset.path)? != asset.stamp {
                return Err("readFailed");
            }
            if request.method() == Method::HEAD {
                return Ok(Vec::new());
            }
            let bytes = image_resource::read(&asset.path, image_resource::IMAGE_BYTES)?;
            let extension = asset
                .path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if image_resource::inspect(&bytes, &extension)? != asset.info
                || Stamp::read(&asset.path)? != asset.stamp
            {
                return Err("readFailed");
            }
            // Recheck ownership after I/O, so an unmounted document cannot start a late decode.
            if self.asset(window, token).is_none() {
                return Err("stale");
            }
            Ok(bytes)
        };
        match content() {
            Ok(bytes) => response(StatusCode::OK, asset.info.mime, bytes, asset.stamp.bytes),
            Err(_) => response(StatusCode::NOT_FOUND, "text/plain", Vec::new(), 0),
        }
    }
}
fn response(status: StatusCode, mime: &str, bytes: Vec<u8>, length: u64) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Content-Length", length.to_string())
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "default-src 'none'; sandbox")
        .body(bytes)
        .unwrap()
}

#[tauri::command]
pub fn local_image_create_scope(
    window: tauri::Webview,
    registry: tauri::State<'_, Registry>,
) -> Result<String, String> {
    registry.create_scope(window.label()).map_err(str::to_owned)
}

#[tauri::command]
pub fn local_image_release_scope(
    window: tauri::Webview,
    registry: tauri::State<'_, Registry>,
    scope: String,
) {
    registry.release_scope(window.label(), &scope);
}

#[tauri::command]
pub async fn local_image_prepare(
    window: tauri::Webview,
    registry: tauri::State<'_, Registry>,
    scope: String,
    path: String,
) -> Result<Prepared, String> {
    let window = window.label().to_owned();
    let registry = registry.inner().clone();
    let permit = registry
        .workers
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "stale")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        registry
            .prepare(&window, &scope, Path::new(&path))
            .map_err(str::to_owned)
    })
    .await
    .map_err(|_| "readFailed".to_owned())?
}

pub fn serve(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    use tauri::Manager;
    let registry = context.app_handle().state::<Registry>().inner().clone();
    let window = context.webview_label().to_owned();
    tauri::async_runtime::spawn(async move {
        let result = if let Ok(permit) = registry.workers.clone().acquire_owned().await {
            tauri::async_runtime::spawn_blocking(move || {
                let _permit = permit;
                registry.respond(&window, &request)
            })
            .await
            .ok()
        } else {
            None
        };
        responder.respond(result.unwrap_or_else(|| {
            response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain",
                Vec::new(),
                0,
            )
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(path: &Path, width: u32, height: u32) {
        image::RgbaImage::new(width, height).save(path).unwrap();
    }
    fn get(token: &str) -> Request<Vec<u8>> {
        Request::builder()
            .uri(format!("{SCHEME}://localhost/{token}"))
            .body(Vec::new())
            .unwrap()
    }
    #[test]
    fn serves_binary_original_only_to_the_owner_and_revokes_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("space 中文.png");
        png(&path, 3, 4);
        let registry = Registry::default();
        let scope = registry.create_scope("popup").unwrap();
        let image = registry.prepare("popup", &scope, &path).unwrap();
        assert_eq!((image.width, image.height), (3, 4));
        let result = registry.respond("popup", &get(&image.token));
        assert_eq!(result.status(), StatusCode::OK);
        assert_eq!(result.headers()["Content-Type"], "image/png");
        assert_eq!(result.body(), &std::fs::read(&path).unwrap());
        assert_eq!(
            registry.respond("history", &get(&image.token)).status(),
            StatusCode::NOT_FOUND
        );
        registry.release_scope("history", &scope);
        assert_eq!(
            registry.respond("popup", &get(&image.token)).status(),
            StatusCode::OK
        );
        registry.release_scope("popup", &scope);
        assert_eq!(
            registry.respond("popup", &get(&image.token)).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            registry.prepare("popup", &scope, &path).unwrap_err(),
            "stale"
        );
    }
    #[test]
    fn rejects_arbitrary_paths_replaced_files_and_non_images() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.png");
        png(&path, 3, 4);
        let registry = Registry::default();
        let scope = registry.create_scope("popup").unwrap();
        let image = registry.prepare("popup", &scope, &path).unwrap();
        assert_eq!(
            registry.respond("popup", &get("../../image.png")).status(),
            StatusCode::NOT_FOUND
        );
        std::fs::write(&path, "changed").unwrap();
        assert_eq!(
            registry.respond("popup", &get(&image.token)).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            registry.prepare("other", &scope, &path).unwrap_err(),
            "stale"
        );
        let bad = temp.path().join("not-image.png");
        std::fs::write(&bad, "text").unwrap();
        assert_eq!(
            registry.prepare("popup", &scope, &bad).unwrap_err(),
            "imageFailed"
        );
    }
    #[test]
    fn rejects_bytes_dimensions_animations_and_total_scope_budget() {
        let temp = tempfile::tempdir().unwrap();
        let large = temp.path().join("large.png");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(image_resource::IMAGE_BYTES as u64 + 1)
            .unwrap();
        let registry = Registry::default();
        let scope = registry.create_scope("popup").unwrap();
        assert_eq!(
            registry.prepare("popup", &scope, &large).unwrap_err(),
            "limit"
        );
        let huge = temp.path().join("huge.svg");
        std::fs::write(&huge, "<svg width='10000' height='10000'/>").unwrap();
        assert_eq!(
            registry.prepare("popup", &scope, &huge).unwrap_err(),
            "limit"
        );
        for i in 0..3 {
            let path = temp.path().join(format!("{i}.svg"));
            std::fs::write(&path, "<svg width='6000' height='6000'/>").unwrap();
            let result = registry.prepare("popup", &scope, &path);
            if i < 2 {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err(), "limit");
            }
        }
        let gif = temp.path().join("many.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(&gif).unwrap());
        for _ in 0..501 {
            encoder
                .encode(&[0, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        drop(encoder);
        assert_eq!(
            registry.prepare("popup", &scope, &gif).unwrap_err(),
            "limit"
        );
    }
    #[test]
    fn reuses_paths_releases_windows_and_preserves_other_scopes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.png");
        png(&path, 3, 4);
        let registry = Registry::default();
        let popup = registry.create_scope("popup").unwrap();
        let history = registry.create_scope("history").unwrap();
        let first = registry.prepare("popup", &popup, &path).unwrap();
        assert_eq!(
            registry.prepare("popup", &popup, &path).unwrap().token,
            first.token
        );
        let second = registry.prepare("history", &history, &path).unwrap();
        registry.release_window("popup");
        assert_eq!(
            registry.respond("popup", &get(&first.token)).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            registry.respond("history", &get(&second.token)).status(),
            StatusCode::OK
        );
    }
    #[test]
    fn supports_head_and_rejects_queries_and_write_methods() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.png");
        png(&path, 3, 4);
        let registry = Registry::default();
        let scope = registry.create_scope("popup").unwrap();
        let image = registry.prepare("popup", &scope, &path).unwrap();
        let mut request = get(&image.token);
        *request.method_mut() = Method::HEAD;
        let response = registry.respond("popup", &request);
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.body().is_empty());
        *request.method_mut() = Method::POST;
        assert_eq!(
            registry.respond("popup", &request).status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            registry
                .respond("popup", &get(&format!("{}?path=secret", image.token)))
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}
