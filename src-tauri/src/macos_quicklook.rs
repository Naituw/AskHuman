//! macOS 原生 QuickLook 预览（QLPreviewPanel），经 objc2 调用。
//!
//! 机制（已用纯 Swift 最小程序实证）：
//! 只有当面板通过【响应链的 QLPreviewPanelController 协议】被控制时，
//! `previewPanel:handleEvent:` 才会被回调；仅设 `delegate` 不会触发它。
//! 因此这里定义一个 **NSResponder 子类** 作为控制者，实现：
//!   - `acceptsPreviewPanelControl:` / `beginPreviewPanelControl:` / `endPreviewPanelControl:`
//!   - DataSource：`numberOfPreviewItemsInPreviewPanel:`（恒为 1，单文件）/ `previewPanel:previewItemAtIndex:`
//!   - Delegate：`previewPanel:handleEvent:`（捕获方向键，改当前文件 + reloadData，并回传索引）
//!
//! 并把该控制者插入弹窗窗口的响应链（`window.nextResponder`）。
//! 这样：焦点在面板（原生），方向键逐个切换单文件预览，弹窗侧据 `preview-index` 同步高亮；
//! 面板关闭时经 `endPreviewPanelControl:` 回传 `preview-closed`。
//! 所有调用都必须在主线程（由调用方经 `run_on_main_thread` 保证）。

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObjectProtocol};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSEventType, NSResponder};
use objc2_foundation::{NSString, NSURL};
use std::cell::{Cell, RefCell};
use tauri::{AppHandle, Emitter, Manager};

const KEY_LEFT: u16 = 123;
const KEY_RIGHT: u16 = 124;
const KEY_DOWN: u16 = 125;
const KEY_UP: u16 = 126;
/// 面板层级：弹窗置顶时为 NSFloatingWindowLevel(3)，用 NSStatusWindowLevel(25) 压在其上。
const NS_STATUS_WINDOW_LEVEL: isize = 25;

struct Ivars {
    urls: RefCell<Vec<Retained<NSURL>>>,
    index: Cell<usize>,
    app: AppHandle,
}

define_class!(
    #[unsafe(super(NSResponder))]
    #[name = "HILQuickLookController"]
    #[ivars = Ivars]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    impl Controller {
        // —— QLPreviewPanelController（响应链）——
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_control(&self, _panel: *mut AnyObject) -> Bool {
            Bool::YES
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_control(&self, panel: *mut AnyObject) {
            unsafe {
                let me: *const Controller = self;
                let _: () = msg_send![panel, setDataSource: me];
                let _: () = msg_send![panel, setDelegate: me];
                // 弹窗置顶时面板会被压住，抬高层级；在成为 key 后设置以防被重置。
                let _: () = msg_send![panel, setLevel: NS_STATUS_WINDOW_LEVEL];
            }
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_control(&self, _panel: *mut AnyObject) {
            invalidate_preparation();
            // 面板关闭：通知前端预览已结束。
            let _ = self.ivars().app.emit("preview-closed", ());
        }

        // —— DataSource（单文件）——
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_items(&self, _panel: *mut AnyObject) -> isize {
            1
        }

        #[unsafe(method(previewPanel:previewItemAtIndex:))]
        fn item_at_index(&self, _panel: *mut AnyObject, _index: isize) -> *mut NSURL {
            let i = self.ivars().index.get();
            let urls = self.ivars().urls.borrow();
            if i >= urls.len() {
                return std::ptr::null_mut();
            }
            // 返回 ivars 持有的 NSURL 的借用指针（+0）：对象生命周期由 ivars.urls 保证，
            // 面板使用期间不会被释放。
            Retained::as_ptr(&urls[i]) as *mut NSURL
        }

        // —— Delegate：捕获方向键 ——
        #[unsafe(method(previewPanel:handleEvent:))]
        fn handle_event(&self, panel: *mut AnyObject, event: &NSEvent) -> Bool {
            Bool::new(unsafe { self.handle_key(panel, event) })
        }
    }
);

impl Controller {
    unsafe fn handle_key(&self, panel: *mut AnyObject, event: &NSEvent) -> bool {
        // 用强类型 NSEvent 绑定取 type/keyCode，选择器映射正确（避免 raw msg_send 的坑）。
        if event.r#type() != NSEventType::KeyDown {
            return false;
        }
        let len = self.ivars().urls.borrow().len();
        if len == 0 {
            return false;
        }
        let code: u16 = event.keyCode();
        let cur = self.ivars().index.get();
        let new = match code {
            KEY_LEFT | KEY_UP => cur.saturating_sub(1),
            KEY_RIGHT | KEY_DOWN => (cur + 1).min(len - 1),
            _ => return false,
        };
        if new != cur {
            // Native navigation wins over a previously queued popup selection.
            invalidate_preparation();
            self.ivars().index.set(new);
            let _: () = msg_send![panel, reloadData];
            let _ = self.ivars().app.emit("preview-index", new);
        }
        true
    }

    fn new(app: AppHandle, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            urls: RefCell::new(Vec::new()),
            index: Cell::new(0),
            app,
        });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// 持久控制者：插入弹窗响应链一次后常驻；每次 show 仅更新其数据。
    static CONTROLLER: RefCell<Option<Retained<Controller>>> = const { RefCell::new(None) };
    /// 标记是否已插入某窗口的响应链，避免重复插入。
    static CHAIN_INSTALLED: Cell<bool> = const { Cell::new(false) };
    // All access is on the main thread. Closing/replacing a preview invalidates pending IO.
    static PREPARATION: Cell<u64> = const { Cell::new(0) };
}

fn invalidate_preparation() -> u64 {
    PREPARATION.with(|generation| {
        let next = generation.get().wrapping_add(1);
        generation.set(next);
        next
    })
}

fn panel_class() -> Option<&'static AnyClass> {
    AnyClass::get(c"QLPreviewPanel")
}

/// 确保控制者存在并已插入 `window` 的响应链。
unsafe fn ensure_controller(app: &AppHandle, window: usize) -> Retained<Controller> {
    let existing = CONTROLLER.with(|c| c.borrow().clone());
    let controller = match existing {
        Some(c) => c,
        None => {
            let mtm =
                MainThreadMarker::new().expect("ensure_controller must run on the main thread");
            let c = Controller::new(app.clone(), mtm);
            CONTROLLER.with(|cell| *cell.borrow_mut() = Some(c.clone()));
            c
        }
    };
    if window != 0 && !CHAIN_INSTALLED.with(|f| f.get()) {
        let win = window as *mut AnyObject;
        // 插入响应链：window -> controller -> (window 原 nextResponder)
        let old: *mut AnyObject = msg_send![win, nextResponder];
        let me: *const Controller = &*controller;
        let _: () = msg_send![win, setNextResponder: me];
        let _: () = msg_send![&*controller, setNextResponder: old];
        CHAIN_INSTALLED.with(|f| f.set(true));
    }
    controller
}

/// 打开预览：展示 `paths[index]` 单个文件；方向键经 handleEvent 逐个联动切换。
pub fn show(app: AppHandle, window: usize, paths: &[String], index: usize) {
    let generation = invalidate_preparation();
    let paths = paths.to_vec();
    tauri::async_runtime::spawn_blocking(move || {
        let prepared: Vec<_> = paths
            .iter()
            .map(|path| effective_preview_path(path))
            .collect();
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if PREPARATION.with(|current| current.get() != generation) {
                return;
            }
            // Never dereference an NSWindow that was destroyed while conversion was running.
            if window != 0
                && !app2.webview_windows().values().any(|win| {
                    win.ns_window()
                        .is_ok_and(|pointer| pointer as usize == window)
                })
            {
                return;
            }
            show_prepared(app2, window, &prepared, index);
        });
    });
}

fn show_prepared(app: AppHandle, window: usize, paths: &[String], index: usize) {
    let urls: Vec<Retained<NSURL>> = paths
        .iter()
        .map(|p| NSURL::fileURLWithPath(&NSString::from_str(p)))
        .collect();
    if urls.is_empty() {
        return;
    }
    let idx = index.min(urls.len() - 1);

    unsafe {
        let controller = ensure_controller(&app, window);
        *controller.ivars().urls.borrow_mut() = urls;
        controller.ivars().index.set(idx);

        let Some(cls) = panel_class() else {
            return;
        };
        let panel: Retained<AnyObject> = msg_send![cls, sharedPreviewPanel];
        // 若已可见（切换选中再预览），直接刷新；否则经响应链控制打开。
        let visible: bool = msg_send![&*panel, isVisible];
        if visible {
            let _: () = msg_send![&*panel, reloadData];
        } else {
            // makeKeyAndOrderFront 会触发系统沿响应链查找控制者 → 回调 begin。
            let null: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![&*panel, makeKeyAndOrderFront: null];
            let _: () = msg_send![&*panel, setLevel: NS_STATUS_WINDOW_LEVEL];
        }
    }
}

/// 取文件的系统图标（Finder 同款，经 NSWorkspace）并编码为 PNG data URL。
/// 必须在主线程调用（由 commands::file_icon_data_url 经 run_on_main_thread 保证）。
pub fn file_icon_png_base64(path: &str) -> Result<String, String> {
    use base64::Engine;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    // NSBitmapImageFileTypePNG = 4；NSCompositingOperationSourceOver = 2。
    const NS_BITMAP_FILE_TYPE_PNG: usize = 4;
    const NS_COMPOSITE_SOURCE_OVER: usize = 2;
    // 拖拽预览图标边长（逻辑像素）。系统图标含 512px 大图，需光栅化到此尺寸避免预览过大。
    const ICON_SIZE: f64 = 64.0;
    unsafe {
        let ws_cls = AnyClass::get(c"NSWorkspace").ok_or("NSWorkspace unavailable")?;
        let ws: *mut AnyObject = msg_send![ws_cls, sharedWorkspace];
        let ns_path = NSString::from_str(path);
        let icon: *mut AnyObject = msg_send![ws, iconForFile: &*ns_path];
        if icon.is_null() {
            return Err("failed to get file icon".into());
        }
        // 将系统图标重绘到固定尺寸的小图，避免 TIFFRepresentation 输出 512px 大图。
        let size = NSSize::new(ICON_SIZE, ICON_SIZE);
        let img_cls = AnyClass::get(c"NSImage").ok_or("NSImage unavailable")?;
        let small: *mut AnyObject = msg_send![img_cls, alloc];
        let small: *mut AnyObject = msg_send![small, initWithSize: size];
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
        let zero = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
        let _: () = msg_send![small, lockFocus];
        let _: () = msg_send![icon, drawInRect: rect, fromRect: zero, operation: NS_COMPOSITE_SOURCE_OVER, fraction: 1.0f64];
        let _: () = msg_send![small, unlockFocus];
        let tiff: *mut AnyObject = msg_send![small, TIFFRepresentation];
        if tiff.is_null() {
            return Err("icon TIFF representation is empty".into());
        }
        let rep_cls = AnyClass::get(c"NSBitmapImageRep").ok_or("NSBitmapImageRep unavailable")?;
        let rep: *mut AnyObject = msg_send![rep_cls, imageRepWithData: tiff];
        if rep.is_null() {
            return Err("bitmap representation is empty".into());
        }
        let dict_cls = AnyClass::get(c"NSDictionary").ok_or("NSDictionary unavailable")?;
        let props: *mut AnyObject = msg_send![dict_cls, dictionary];
        let png: *mut AnyObject =
            msg_send![rep, representationUsingType: NS_BITMAP_FILE_TYPE_PNG, properties: props];
        if png.is_null() {
            return Err("PNG encoding failed".into());
        }
        let len: usize = msg_send![png, length];
        let bytes: *const std::ffi::c_void = msg_send![png, bytes];
        if bytes.is_null() || len == 0 {
            return Err("PNG data is empty".into());
        }
        let slice = std::slice::from_raw_parts(bytes as *const u8, len);
        let b64 = base64::engine::general_purpose::STANDARD.encode(slice);
        Ok(format!("data:image/png;base64,{}", b64))
    }
}

/// 已识别的 Markdown 扩展名（小写、含点）。
const MARKDOWN_EXTS: [&str; 5] = [".md", ".markdown", ".mdown", ".mkd", ".mdwn"];

/// Convert supported text attachments to static HTML; preserve the original on write failure.
fn effective_preview_path(path: &str) -> String {
    if crate::attachment_diff::supports(path) {
        return write_preview_html(path, &crate::attachment_diff::render_file(path))
            .unwrap_or_else(|| path.to_string());
    }
    let lower = path.to_ascii_lowercase();
    if !MARKDOWN_EXTS.iter().any(|e| lower.ends_with(e)) {
        return path.to_string();
    }
    render_markdown_to_temp_html(path).unwrap_or_else(|| path.to_string())
}

/// 读取 Markdown 文件、渲染为自包含 HTML（内联 CSS、深浅色自适应）写入临时文件，返回其路径。
/// 临时文件落在 `temp/askhuman/preview/<uuid>/<原名>.html`：纳入 daemon 的 24h temp 清理；
/// 独立子目录避免同名附件相互覆盖，并让标题栏显示「<原名>.html」（而非随机串）。
fn render_markdown_to_temp_html(path: &str) -> Option<String> {
    let src = std::fs::read_to_string(path).ok()?;
    let title = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("preview");
    let doc = wrap_html(title, &markdown_to_html(&src));
    write_preview_html(path, &doc)
}

fn write_preview_html(path: &str, doc: &str) -> Option<String> {
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("preview");
    let dir = std::env::temp_dir()
        .join("askhuman")
        .join("preview")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join(format!("{}.html", stem));
    std::fs::write(&out, doc.as_bytes()).ok()?;
    Some(out.to_string_lossy().into_owned())
}

/// Markdown → HTML 片段。启用常用 GFM 扩展；**原始 HTML 一律转义为文本**（对齐前端
/// markdown-it 的 `html:false`，避免本地预览里执行任意标记）。
fn markdown_to_html(src: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(src, opts).map(|ev| match ev {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// 包成自包含 HTML 文档：内联样式 + 深浅色随系统（QuickLook 用 WebKit 渲染、遵循系统外观）。
fn wrap_html(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title><style>{css}</style></head>\
<body><article class=\"markdown-body\">{body}</article></body></html>",
        title = escape_html_min(title),
        css = PREVIEW_CSS,
        body = body,
    )
}

/// 最小 HTML 转义（仅用于 `<title>`）。
fn escape_html_min(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 预览样式：简洁 GitHub 风，深浅色随系统。
const PREVIEW_CSS: &str = r#"
:root { color-scheme: light dark; }
body {
  margin: 0;
  background: #ffffff;
  color: #1f2328;
  font: 15px/1.65 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Arial, "PingFang SC", "Hiragino Sans GB", sans-serif;
}
.markdown-body { max-width: 820px; margin: 0 auto; padding: 28px 32px 48px; word-wrap: break-word; }
.markdown-body h1, .markdown-body h2 { border-bottom: 1px solid #d8dee4; padding-bottom: .3em; }
.markdown-body h1 { font-size: 1.9em; } .markdown-body h2 { font-size: 1.5em; }
.markdown-body h3 { font-size: 1.25em; } .markdown-body h4 { font-size: 1em; }
.markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4, .markdown-body h5, .markdown-body h6 {
  margin: 1.4em 0 .6em; font-weight: 600; line-height: 1.3;
}
.markdown-body p, .markdown-body ul, .markdown-body ol, .markdown-body blockquote, .markdown-body table, .markdown-body pre { margin: 0 0 1em; }
.markdown-body a { color: #0969da; text-decoration: none; }
.markdown-body a:hover { text-decoration: underline; }
.markdown-body code {
  font: .88em/1.5 ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  background: rgba(129,139,152,.18); padding: .2em .4em; border-radius: 6px;
}
.markdown-body pre {
  background: #f6f8fa; padding: 14px 16px; border-radius: 8px; overflow: auto;
}
.markdown-body pre code { background: none; padding: 0; }
.markdown-body blockquote { color: #59636e; border-left: .25em solid #d0d7de; padding: 0 1em; }
.markdown-body table { border-collapse: collapse; display: block; overflow: auto; }
.markdown-body th, .markdown-body td { border: 1px solid #d0d7de; padding: 6px 13px; }
.markdown-body tr:nth-child(2n) { background: #f6f8fa; }
.markdown-body img { max-width: 100%; }
.markdown-body hr { border: 0; border-top: 1px solid #d8dee4; margin: 1.6em 0; }
.markdown-body ul.contains-task-list { list-style: none; padding-left: 1.2em; }
.markdown-body li input[type=checkbox] { margin: 0 .4em 0 -1.2em; }
@media (prefers-color-scheme: dark) {
  body { background: #1e1e1e; color: #e6edf3; }
  .markdown-body h1, .markdown-body h2 { border-bottom-color: #3d444d; }
  .markdown-body a { color: #4493f8; }
  .markdown-body code { background: rgba(101,108,118,.32); }
  .markdown-body pre { background: #161b22; }
  .markdown-body blockquote { color: #9198a1; border-left-color: #3d444d; }
  .markdown-body th, .markdown-body td { border-color: #3d444d; }
  .markdown-body tr:nth-child(2n) { background: #161b22; }
  .markdown-body hr { border-top-color: #3d444d; }
}
"#;

/// 关闭当前预览面板（若存在且可见）。
pub fn hide() {
    invalidate_preparation();
    let Some(cls) = panel_class() else {
        return;
    };
    unsafe {
        let exists: bool = msg_send![cls, sharedPreviewPanelExists];
        if !exists {
            return;
        }
        let panel: Retained<AnyObject> = msg_send![cls, sharedPreviewPanel];
        let visible: bool = msg_send![&*panel, isVisible];
        if visible {
            let null: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![&*panel, orderOut: null];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_attachment_becomes_html_without_changing_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.PATCH");
        let src = include_str!("../tests/fixtures/attachment-preview.patch");
        std::fs::write(&path, src).unwrap();
        let converted = effective_preview_path(path.to_str().unwrap());
        assert!(converted.ends_with("review.html"));
        let html = std::fs::read_to_string(&converted).unwrap();
        assert!(html.contains("class=\"line add\""));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), src);
        std::fs::remove_dir_all(std::path::Path::new(&converted).parent().unwrap()).unwrap();
    }

    #[test]
    fn markdown_still_renders_and_other_attachments_keep_their_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "# Title\n\n<script>hello</script>").unwrap();
        let converted = effective_preview_path(path.to_str().unwrap());
        let html = std::fs::read_to_string(&converted).unwrap();
        assert!(html.contains("<h1>Title</h1>"));
        assert!(!html.contains("<script>"));
        std::fs::remove_dir_all(std::path::Path::new(&converted).parent().unwrap()).unwrap();
        let other = dir.path().join("plain.txt");
        assert_eq!(
            effective_preview_path(other.to_str().unwrap()),
            other.to_str().unwrap()
        );
    }
}
