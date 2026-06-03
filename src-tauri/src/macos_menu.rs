//! macOS 原生右键菜单（NSMenu），组织/命名贴近 Finder，作用于 -f 附件胶囊。
//!
//! 菜单项（Finder 风格）：
//!   打开 / 打开方式 ▸（可打开该文件的应用列表，带图标；末尾「其他…」走系统选择器）
//!   ——
//!   快速查看「name」/ 在访达中显示
//!   ——
//!   拷贝「name」（文件入剪贴板）/ 拷贝路径
//!
//! 全程主线程（由 commands::show_attachment_menu 经 run_on_main_thread 保证）。
//! 实现以原生 NSMenu/NSWorkspace/NSPasteboard 为主，多数调用走 raw msg_send，
//! 仅菜单项的 target 用 define_class 定义（NSMenuItem.target 为弱引用，故弹出期间须持活）。

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker};
use objc2_foundation::{NSArray, NSPoint, NSString, NSURL};
use std::cell::RefCell;
use tauri::{AppHandle, Manager};

// 菜单项 tag → 动作。100+ 表示「打开方式」中的第 N 个应用。
const TAG_OPEN: isize = 1;
const TAG_REVEAL: isize = 2;
const TAG_QUICKLOOK: isize = 3;
const TAG_COPY_FILE: isize = 4;
const TAG_COPY_PATH: isize = 5;
const TAG_OPEN_WITH_OTHER: isize = 99;
const TAG_APP_BASE: isize = 100;

const NS_MODAL_RESPONSE_OK: isize = 1;

struct Ivars {
    app: AppHandle,
    path: String,
    app_urls: RefCell<Vec<Retained<NSURL>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HILMenuTarget"]
    #[ivars = Ivars]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(menuAction:))]
        fn menu_action(&self, sender: *mut AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            unsafe { self.perform(tag) };
        }
    }
);

impl Target {
    fn new(app: AppHandle, path: String) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            app,
            path,
            app_urls: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }

    unsafe fn perform(&self, tag: isize) {
        let path = self.ivars().path.clone();
        let ws = workspace();
        let url = file_url(&path);
        match tag {
            TAG_OPEN => {
                let _: bool = msg_send![ws, openURL: &*url];
            }
            TAG_REVEAL => {
                let arr = NSArray::from_slice(&[&*url]);
                let _: () = msg_send![ws, activateFileViewerSelectingURLs: &*arr];
            }
            TAG_QUICKLOOK => {
                let win = self
                    .ivars()
                    .app
                    .get_webview_window("popup")
                    .and_then(|w| w.ns_window().ok())
                    .map(|p| p as usize)
                    .unwrap_or(0);
                crate::macos_quicklook::show(self.ivars().app.clone(), win, &[path], 0);
            }
            TAG_COPY_FILE => {
                let pb = general_pasteboard();
                let _: () = msg_send![pb, clearContents];
                let arr = NSArray::from_slice(&[&*url]);
                let _: bool = msg_send![pb, writeObjects: &*arr];
            }
            TAG_COPY_PATH => {
                let pb = general_pasteboard();
                let _: () = msg_send![pb, clearContents];
                let kind = NSString::from_str("public.utf8-plain-text");
                let types = NSArray::from_slice(&[&*kind]);
                let null: *mut AnyObject = std::ptr::null_mut();
                let _: isize = msg_send![pb, declareTypes: &*types, owner: null];
                let s = NSString::from_str(&path);
                let _: bool = msg_send![pb, setString: &*s, forType: &*kind];
            }
            TAG_OPEN_WITH_OTHER => self.open_with_other(),
            t if t >= TAG_APP_BASE => {
                let idx = (t - TAG_APP_BASE) as usize;
                let apps = self.ivars().app_urls.borrow();
                if let Some(app_url) = apps.get(idx) {
                    open_with(&path, app_url);
                }
            }
            _ => {}
        }
    }

    /// 「其他…」：弹系统选择器（限定 /Applications 下的 .app）选应用打开。
    unsafe fn open_with_other(&self) {
        let panel_cls = match AnyClass::get(c"NSOpenPanel") {
            Some(c) => c,
            None => return,
        };
        let panel: *mut AnyObject = msg_send![panel_cls, openPanel];
        let _: () = msg_send![panel, setCanChooseFiles: true];
        let _: () = msg_send![panel, setCanChooseDirectories: false];
        let _: () = msg_send![panel, setAllowsMultipleSelection: false];
        let apps_dir = file_url("/Applications");
        let _: () = msg_send![panel, setDirectoryURL: &*apps_dir];
        let exts = NSString::from_str("app");
        let types = NSArray::from_slice(&[&*exts]);
        let _: () = msg_send![panel, setAllowedFileTypes: &*types];
        let resp: isize = msg_send![panel, runModal];
        if resp != NS_MODAL_RESPONSE_OK {
            return;
        }
        let urls: *mut AnyObject = msg_send![panel, URLs];
        let count: usize = msg_send![urls, count];
        if count == 0 {
            return;
        }
        let app_url: *mut NSURL = msg_send![urls, objectAtIndex: 0usize];
        let app_url = Retained::retain(app_url).unwrap();
        open_with(&self.ivars().path, &app_url);
    }
}

fn workspace() -> *mut AnyObject {
    unsafe {
        let cls = AnyClass::get(c"NSWorkspace").expect("NSWorkspace");
        msg_send![cls, sharedWorkspace]
    }
}

fn general_pasteboard() -> *mut AnyObject {
    unsafe {
        let cls = AnyClass::get(c"NSPasteboard").expect("NSPasteboard");
        msg_send![cls, generalPasteboard]
    }
}

fn file_url(path: &str) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(path))
}

/// 用指定应用打开文件（采用稳定的 openFile:withApplication:）。
unsafe fn open_with(path: &str, app_url: &NSURL) {
    let ws = workspace();
    let app_path: *mut NSString = msg_send![app_url, path];
    if app_path.is_null() {
        return;
    }
    let file = NSString::from_str(path);
    let _: bool = msg_send![ws, openFile: &*file, withApplication: app_path];
}

/// 取文件的「可打开应用」列表（macOS 12+：URLsForApplicationsToOpenURL:）。
unsafe fn apps_for_file(path: &str) -> Vec<Retained<NSURL>> {
    let ws = workspace();
    let url = file_url(path);
    let arr: *mut AnyObject = msg_send![ws, URLsForApplicationsToOpenURL: &*url];
    if arr.is_null() {
        return Vec::new();
    }
    let count: usize = msg_send![arr, count];
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let u: *mut NSURL = msg_send![arr, objectAtIndex: i];
        if !u.is_null() {
            if let Some(r) = Retained::retain(u) {
                out.push(r);
            }
        }
    }
    out
}

/// 应用 URL 的显示名（去掉 .app 后缀的 bundle 名）。
unsafe fn app_display_name(app_url: &NSURL) -> String {
    let last: *mut NSString = msg_send![app_url, lastPathComponent];
    if last.is_null() {
        return "应用".to_string();
    }
    let name = (*last).to_string();
    name.strip_suffix(".app").unwrap_or(&name).to_string()
}

/// 应用图标（16×16）作为菜单项左侧小图标。
unsafe fn app_icon_16(app_url: &NSURL) -> *mut AnyObject {
    let ws = workspace();
    let p: *mut NSString = msg_send![app_url, path];
    if p.is_null() {
        return std::ptr::null_mut();
    }
    let icon: *mut AnyObject = msg_send![ws, iconForFile: p];
    if icon.is_null() {
        return icon;
    }
    let size = objc2_foundation::NSSize::new(16.0, 16.0);
    let _: () = msg_send![icon, setSize: size];
    icon
}

unsafe fn new_menu() -> *mut AnyObject {
    let cls = AnyClass::get(c"NSMenu").expect("NSMenu");
    let m: *mut AnyObject = msg_send![cls, alloc];
    let m: *mut AnyObject = msg_send![m, init];
    let _: () = msg_send![m, setAutoenablesItems: false];
    m
}

unsafe fn add_item(
    menu: *mut AnyObject,
    title: &str,
    tag: isize,
    target: &Target,
) -> *mut AnyObject {
    let cls = AnyClass::get(c"NSMenuItem").expect("NSMenuItem");
    let item: *mut AnyObject = msg_send![cls, alloc];
    let t = NSString::from_str(title);
    let empty = NSString::from_str("");
    let item: *mut AnyObject =
        msg_send![item, initWithTitle: &*t, action: sel!(menuAction:), keyEquivalent: &*empty];
    let _: () = msg_send![item, setTag: tag];
    let target_obj: *const Target = target;
    let _: () = msg_send![item, setTarget: target_obj];
    let _: () = msg_send![item, setEnabled: true];
    let _: () = msg_send![menu, addItem: item];
    item
}

unsafe fn add_separator(menu: *mut AnyObject) {
    let cls = AnyClass::get(c"NSMenuItem").expect("NSMenuItem");
    let sep: *mut AnyObject = msg_send![cls, separatorItem];
    let _: () = msg_send![menu, addItem: sep];
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// 构建并弹出附件的右键菜单。
pub fn show(app: AppHandle, path: String) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let name = basename(&path);
    let target = Target::new(app, path.clone());

    unsafe {
        let menu = new_menu();

        // 打开
        add_item(menu, "打开", TAG_OPEN, &target);

        // 打开方式 ▸
        let apps = apps_for_file(&path);
        let submenu = new_menu();
        for (i, app_url) in apps.iter().enumerate() {
            let item = add_item(
                submenu,
                &app_display_name(app_url),
                TAG_APP_BASE + i as isize,
                &target,
            );
            let icon = app_icon_16(app_url);
            if !icon.is_null() {
                let _: () = msg_send![item, setImage: icon];
            }
        }
        if !apps.is_empty() {
            add_separator(submenu);
        }
        add_item(submenu, "其他…", TAG_OPEN_WITH_OTHER, &target);
        *target.ivars().app_urls.borrow_mut() = apps;

        let cls = AnyClass::get(c"NSMenuItem").expect("NSMenuItem");
        let ow_item: *mut AnyObject = msg_send![cls, alloc];
        let ow_title = NSString::from_str("打开方式");
        let empty = NSString::from_str("");
        let null: *mut AnyObject = std::ptr::null_mut();
        let ow_item: *mut AnyObject =
            msg_send![ow_item, initWithTitle: &*ow_title, action: null, keyEquivalent: &*empty];
        let _: () = msg_send![ow_item, setSubmenu: submenu];
        let _: () = msg_send![menu, addItem: ow_item];

        add_separator(menu);
        add_item(menu, &format!("快速查看「{}」", name), TAG_QUICKLOOK, &target);
        add_item(menu, "在访达中显示", TAG_REVEAL, &target);

        add_separator(menu);
        add_item(menu, &format!("拷贝「{}」", name), TAG_COPY_FILE, &target);
        add_item(menu, "拷贝路径", TAG_COPY_PATH, &target);

        // 在当前鼠标位置（屏幕坐标）弹出；inView 传 nil 即按屏幕坐标定位。
        let event_cls = AnyClass::get(c"NSEvent").expect("NSEvent");
        let loc: NSPoint = msg_send![event_cls, mouseLocation];
        let nil_item: *mut AnyObject = std::ptr::null_mut();
        let nil_view: *mut AnyObject = std::ptr::null_mut();
        let _: bool =
            msg_send![menu, popUpMenuPositioningItem: nil_item, atLocation: loc, inView: nil_view];
    }
    // target 在此前一直被栈持有；popUp 为模态，动作在返回前同步执行完。
    drop(target);
}
