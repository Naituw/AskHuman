//! 让弹窗使用 macOS 原生窗口出现动画（缩放 + 淡入）。
//!
//! 关键：`NSWindowAnimationBehavior` 的取值为
//! Default=0、None=2、DocumentWindow=3、UtilityWindow=4、AlertPanel=5。
//! （注意 2 是 None，会禁用动画。）具体取值由用户在设置页选择。
//!
//! 用法：窗口需「隐藏构建」，设好 animationBehavior 后再 `show()`，
//! 这样 `orderFront` 才会播放系统出现动画。

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use std::ffi::c_void;

/// 给 `NSWindow` 设置原生出现动画行为；`behavior` 为 `NSWindowAnimationBehavior` 原始取值。
/// `ns_window` 来自 `WebviewWindow::ns_window()`；为空则忽略。
pub fn set_appear_animation(ns_window: *mut c_void, behavior: isize) {
    if ns_window.is_null() {
        return;
    }
    let win = ns_window as *mut AnyObject;
    unsafe {
        let _: () = msg_send![win, setAnimationBehavior: behavior];
    }
}

/// 移除窗口 contentView 下所有 `NSVisualEffectView`（Tauri 的 vibrancy 背景）。
/// Tauri 的 `set_effects(None)` 在 macOS 上是空实现，切回 Liquid Glass 前需手动清掉，
/// 否则残留的 vibrancy 会盖在玻璃层之上。`ns_window` 为空则忽略。
pub fn remove_vibrancy_views(ns_window: *mut c_void) {
    if ns_window.is_null() {
        return;
    }
    let Some(cls) = AnyClass::get(c"NSVisualEffectView") else {
        return;
    };
    let win = ns_window as *mut AnyObject;
    unsafe {
        let content: *mut AnyObject = msg_send![win, contentView];
        if content.is_null() {
            return;
        }
        let subviews: *mut AnyObject = msg_send![content, subviews];
        if subviews.is_null() {
            return;
        }
        let count: usize = msg_send![subviews, count];
        // 先收集再移除，避免遍历期间修改 subviews。
        let mut targets: Vec<*mut AnyObject> = Vec::new();
        for i in 0..count {
            let view: *mut AnyObject = msg_send![subviews, objectAtIndex: i];
            if view.is_null() {
                continue;
            }
            let is_kind: bool = msg_send![view, isKindOfClass: cls];
            if is_kind {
                targets.push(view);
            }
        }
        for view in targets {
            let _: () = msg_send![view, removeFromSuperview];
        }
    }
}
