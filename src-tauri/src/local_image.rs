//! Window-scoped image URLs. Only registered, bounded images can be served by this protocol.
use crate::image_resource::{self, ImageInfo};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
use tauri::http::{Method, Request, Response, StatusCode};

pub const SCHEME: &str = "askhuman-image";
const MAX_SCOPES: usize = 256;
const MAX_IMAGES_PER_SCOPE: usize = 128;
const MAX_SCOPE_PIXELS: u64 = image_resource::ANIMATION_PIXELS;
const GENERATED_PREFIX: &str = "askhuman-preview-images-";

struct GeneratedStore {
    _lock: std::fs::File,
    dir: tempfile::TempDir,
}
fn owner_id(metadata: &std::fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(metadata.uid())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}
fn cleanup_generated_dirs(root: &Path, now: SystemTime, owner: Option<u32>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(GENERATED_PREFIX)
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if owner_id(&metadata) != owner
            || metadata
                .modified()
                .ok()
                .and_then(|stamp| now.duration_since(stamp).ok())
                .is_none_or(|age| age.as_secs() < 24 * 60 * 60)
        {
            continue;
        }
        let Ok(lock) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(entry.path().join(".lock"))
        else {
            continue;
        };
        // Never reap a different process's live preview files, even after a long request.
        if fs2::FileExt::try_lock_exclusive(&lock).is_err() {
            continue;
        }
        drop(lock);
        let _ = std::fs::remove_dir_all(entry.path());
    }
}
impl GeneratedStore {
    fn create() -> Result<Self, &'static str> {
        let dir = tempfile::Builder::new()
            .prefix(GENERATED_PREFIX)
            .tempdir()
            .map_err(|_| "readFailed")?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(dir.path().join(".lock"))
            .map_err(|_| "readFailed")?;
        fs2::FileExt::lock_exclusive(&lock).map_err(|_| "readFailed")?;
        let owner = dir
            .path()
            .metadata()
            .ok()
            .and_then(|metadata| owner_id(&metadata));
        cleanup_generated_dirs(&std::env::temp_dir(), SystemTime::now(), owner);
        Ok(Self { _lock: lock, dir })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    bytes: u64,
    modified: Option<SystemTime>,
}
impl Stamp {
    pub fn read(path: &Path) -> Result<Self, &'static str> {
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
    references: usize,
    generated: Option<Arc<tempfile::NamedTempFile>>,
    byte_length: u64,
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
    generated_store: Arc<Mutex<Option<Arc<GeneratedStore>>>>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            scopes: Default::default(),
            workers: Arc::new(tokio::sync::Semaphore::new(1)),
            generated_store: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub token: String,
    pub width: u32,
    pub height: u32,
    pub byte_length: u64,
    pub display_pixels: u64,
}
impl Registry {
    fn generated_store(&self) -> Result<Arc<GeneratedStore>, &'static str> {
        let mut store = self.generated_store.lock().unwrap();
        if store.is_none() {
            *store = Some(Arc::new(GeneratedStore::create()?));
        }
        Ok(store.as_ref().unwrap().clone())
    }
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
    pub fn release_asset(&self, window: &str, scope: &str, token: &str) {
        let mut scopes = self.scopes.lock().unwrap();
        let Some(scope) = scopes.get_mut(scope).filter(|s| s.window == window) else {
            return;
        };
        let Some(asset) = scope.assets.get_mut(token) else {
            return;
        };
        asset.references = asset.references.saturating_sub(1);
        if asset.references == 0 {
            scope.pixels = scope.pixels.saturating_sub(asset.info.display_pixels);
            scope.assets.remove(token);
        }
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
            let mut scopes = self.scopes.lock().unwrap();
            let scope = scopes
                .get_mut(id)
                .filter(|s| s.window == window)
                .ok_or("stale")?;
            if let Some((token, asset)) = scope.assets.iter_mut().find(|(_, a)| a.path == path) {
                asset.references = asset.references.saturating_add(1);
                return Ok(Prepared {
                    token: token.clone(),
                    width: asset.info.width,
                    height: asset.info.height,
                    byte_length: asset.byte_length,
                    display_pixels: asset.info.display_pixels,
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
        scope.assets.insert(
            token.clone(),
            Asset {
                path,
                stamp,
                info,
                references: 1,
                generated: None,
                byte_length: stamp.bytes,
            },
        );
        Ok(Prepared {
            token,
            width: info.width,
            height: info.height,
            byte_length: stamp.bytes,
            display_pixels: info.display_pixels,
        })
    }
    pub fn prepare_generated(
        &self,
        window: &str,
        id: &str,
        source: &Path,
        stamp: Stamp,
        bytes: &[u8],
    ) -> Result<Prepared, &'static str> {
        if !self.active(window, id) {
            return Err("stale");
        }
        if bytes.len() > image_resource::IMAGE_BYTES {
            return Err("limit");
        }
        let info = image_resource::inspect(bytes, "png")?;
        if info.mime != "image/png" {
            return Err("imageFailed");
        }
        let source = source.canonicalize().map_err(|_| "readFailed")?;
        if Stamp::read(&source)? != stamp {
            return Err("readFailed");
        }
        let store = self.generated_store()?;
        let mut file =
            tempfile::NamedTempFile::new_in(store.dir.path()).map_err(|_| "readFailed")?;
        file.write_all(bytes).map_err(|_| "readFailed")?;
        if Stamp::read(&source)? != stamp {
            return Err("readFailed");
        }
        let byte_length = bytes.len() as u64;
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
        scope.assets.insert(
            token.clone(),
            Asset {
                path: source,
                stamp,
                info,
                references: 1,
                generated: Some(Arc::new(file)),
                byte_length,
            },
        );
        Ok(Prepared {
            token,
            width: info.width,
            height: info.height,
            byte_length,
            display_pixels: info.display_pixels,
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
            let content_path = asset
                .generated
                .as_ref()
                .map(|file| file.path())
                .unwrap_or(&asset.path);
            let bytes = image_resource::read(content_path, image_resource::IMAGE_BYTES)?;
            let extension = content_path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let extension = if asset.generated.is_some() {
                "png"
            } else {
                &extension
            };
            if image_resource::inspect(&bytes, extension)? != asset.info
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
            Ok(bytes) => response(StatusCode::OK, asset.info.mime, bytes, asset.byte_length),
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
pub fn local_image_release_asset(
    window: tauri::Webview,
    registry: tauri::State<'_, Registry>,
    scope: String,
    token: String,
) {
    registry.release_asset(window.label(), &scope, &token);
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
    fn releases_shared_assets_only_after_the_last_reference() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.png");
        png(&path, 3, 4);
        let registry = Registry::default();
        let scope = registry.create_scope("history").unwrap();
        let other = registry.create_scope("history").unwrap();
        let first = registry.prepare("history", &scope, &path).unwrap();
        #[cfg(unix)]
        let alias = temp.path().join("alias.png");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        #[cfg(not(unix))]
        let alias = path.clone();
        let second = registry.prepare("history", &scope, &alias).unwrap();
        assert_eq!(first.token, second.token);
        registry.release_asset("popup", &scope, &first.token);
        registry.release_asset("history", &other, &first.token);
        registry.release_asset("history", &scope, &first.token);
        assert_eq!(
            registry.respond("history", &get(&first.token)).status(),
            StatusCode::OK
        );
        registry.release_asset("history", &scope, &second.token);
        assert_eq!(
            registry.respond("history", &get(&first.token)).status(),
            StatusCode::NOT_FOUND
        );
    }
    #[test]
    fn refunds_the_scope_pixel_budget_when_an_asset_is_removed() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        let scope = registry.create_scope("todos").unwrap();
        let paths: Vec<_> = (0..3)
            .map(|i| {
                let path = temp.path().join(format!("{i}.svg"));
                std::fs::write(&path, "<svg width='6000' height='6000'/>").unwrap();
                path
            })
            .collect();
        let first = registry.prepare("todos", &scope, &paths[0]).unwrap();
        registry.prepare("todos", &scope, &paths[1]).unwrap();
        assert_eq!(
            registry.prepare("todos", &scope, &paths[2]).unwrap_err(),
            "limit"
        );
        registry.release_asset("todos", &scope, &first.token);
        assert!(registry.prepare("todos", &scope, &paths[2]).is_ok());
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
    #[test]
    fn generated_pngs_serve_binary_bytes_and_delete_the_temporary_file_on_release() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.tga");
        std::fs::write(&source, b"source").unwrap();
        let png_file = temp.path().join("converted.png");
        png(&png_file, 10, 8);
        let bytes = std::fs::read(png_file).unwrap();
        let registry = Registry::default();
        let scope = registry.create_scope("popup").unwrap();
        let prepared = registry
            .prepare_generated(
                "popup",
                &scope,
                &source,
                Stamp::read(&source).unwrap(),
                &bytes,
            )
            .unwrap();
        let generated = registry
            .asset("popup", &prepared.token)
            .unwrap()
            .generated
            .as_ref()
            .unwrap()
            .path()
            .to_path_buf();
        assert!(generated.exists());
        let response = registry.respond("popup", &get(&prepared.token));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["Content-Type"], "image/png");
        assert_eq!(
            response.headers()["Content-Length"],
            bytes.len().to_string()
        );
        assert_eq!(response.body(), &bytes);
        assert_eq!(
            (prepared.width, prepared.height, prepared.display_pixels),
            (10, 8, 80)
        );
        registry.release_scope("popup", &scope);
        assert!(!generated.exists());
        assert_eq!(
            registry.respond("popup", &get(&prepared.token)).status(),
            StatusCode::NOT_FOUND
        );
    }
    #[test]
    fn generated_images_reject_changed_sources_invalid_bytes_and_cancelled_scopes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.ico");
        std::fs::write(&source, b"source").unwrap();
        let stamp = Stamp::read(&source).unwrap();
        let png_file = temp.path().join("converted.png");
        png(&png_file, 2, 3);
        let bytes = std::fs::read(png_file).unwrap();
        let registry = Registry::default();
        let scope = registry.create_scope("popup").unwrap();
        assert!(registry
            .prepare_generated("popup", &scope, &source, stamp, b"not png")
            .is_err());
        std::fs::write(&source, b"changed source").unwrap();
        assert_eq!(
            registry
                .prepare_generated("popup", &scope, &source, stamp, &bytes)
                .unwrap_err(),
            "readFailed"
        );
        registry.release_scope("popup", &scope);
        assert_eq!(
            registry
                .prepare_generated("popup", &scope, &source, stamp, &bytes)
                .unwrap_err(),
            "stale"
        );
    }
    #[test]
    fn orphan_cleanup_preserves_live_locked_stores() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(format!("{GENERATED_PREFIX}live"));
        std::fs::create_dir(&dir).unwrap();
        let lock = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(dir.join(".lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let owner = owner_id(&dir.metadata().unwrap());
        let later = SystemTime::now() + std::time::Duration::from_secs(48 * 60 * 60);
        cleanup_generated_dirs(root.path(), later, owner);
        assert!(dir.exists());
        drop(lock);
        cleanup_generated_dirs(root.path(), later, owner);
        assert!(!dir.exists());
    }
}
