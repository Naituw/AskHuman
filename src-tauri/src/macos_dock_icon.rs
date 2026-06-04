//! 运行时设置 Dock / Cmd+Tab 图标。
//!
//! 裸二进制（非 .app 包）运行时不会读取 bundle 的 icon.icns，Dock 会显示通用图标。
//! 这里在 GUI 启动（主线程）时，把内嵌的 PNG 设为 `NSApplication.applicationIconImage`，
//! 覆盖当前进程的 Dock 图标（同时影响 Cmd+Tab 切换器）。
//!
//! 注意：仅影响「运行中的进程」，不改变 bundle 身份；macOS 不会自动加圆角，
//! 形状/留白完全取决于所给 PNG（见 icons/icon.png）。

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage, NSRequestUserAttentionType};
use objc2_foundation::{NSData, NSString};

/// 内嵌的图标位图。替换 `icons/icon.png`（建议 1024×1024、透明背景、已做 squircle 圆角+留白）即可换图。
const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

/// 把内嵌 PNG 设为当前进程的 Dock 图标。必须在主线程调用（Tauri `setup` 闭包即主线程）。
/// 任一步失败都静默返回，不影响弹窗主流程。
pub fn set_dock_icon() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(ICON_PNG);
    let Some(image): Option<Retained<NSImage>> = NSImage::initWithData(NSImage::alloc(), &data)
    else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

/// 弹窗出现时引起注意：Dock 图标跳动（通知式）+ 角标显示提问个数。
/// 仅 popup 模式调用。注意：App 已是活跃应用时系统不会跳动。
pub fn announce_questions(count: usize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);

    // Dock 角标：显示待回答的提问个数。
    let badge = NSString::from_str(&count.to_string());
    app.dockTile().setBadgeLabel(Some(&badge));

    // 通知式跳动：CriticalRequest 会持续跳动直到 App 被激活。
    app.requestUserAttention(NSRequestUserAttentionType::CriticalRequest);
}
