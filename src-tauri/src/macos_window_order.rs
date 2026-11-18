//! Native AppKit helpers for popup Spaces behavior and window ordering.

use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::{NSPoint, NSRect};
use std::ffi::c_void;

/// Keep a popup eligible for Dock activation across Spaces even at a floating window level.
///
/// # Safety
/// Call on the main thread with the native NSWindow pointer of a live popup, or null.
pub unsafe fn manage_in_spaces(ns_window: *mut c_void) {
    let Some(window) = (unsafe { ns_window.cast::<NSWindow>().as_ref() }) else {
        return;
    };
    window.setCollectionBehavior(managed_collection_behavior(window.collectionBehavior()));
}

fn managed_collection_behavior(
    mut behavior: NSWindowCollectionBehavior,
) -> NSWindowCollectionBehavior {
    // Floating windows default to Transient. Explicit Managed behavior survives level changes.
    behavior.remove(NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::Stationary);
    behavior.insert(NSWindowCollectionBehavior::Managed);
    behavior
}

/// Return the process-global AppKit window number used for relative cross-process ordering.
pub fn window_number(ns_window: *mut c_void) -> Option<i64> {
    if ns_window.is_null() {
        return None;
    }
    let window = ns_window as *mut AnyObject;
    let number: isize = unsafe { msg_send![window, windowNumber] };
    (number > 0).then_some(number as i64)
}

/// Apply AppKit's standard diagonal cascade repeatedly from the centered window position.
/// AppKit handles the visible screen frame and returns the anchor for the next cascade slot.
pub fn cascade(ns_window: *mut c_void, cascade_index: u32) {
    if ns_window.is_null() || cascade_index == 0 {
        return;
    }
    let window = ns_window as *mut AnyObject;
    unsafe {
        let frame: NSRect = msg_send![window, frame];
        let mut next = NSPoint::new(frame.origin.x, frame.origin.y + frame.size.height);
        // The first call places the receiver at the supplied anchor and returns the next slot.
        // One additional call is therefore required for cascade index 1 to visibly move.
        for _ in 0..=cascade_index {
            next = msg_send![window, cascadeTopLeftFromPoint: next];
        }
    }
}

/// Order a hidden popup behind another popup without making it key or activating NSApp.
///
/// `NSWindowBelow` is -1. AppKit window numbers are global, so `relativeTo` can reference a
/// popup owned by another short-lived helper process. If no predecessor is available, orderBack
/// is the conservative non-activating fallback.
pub fn show_behind(ns_window: *mut c_void, behind_window_number: Option<i64>) {
    if ns_window.is_null() {
        return;
    }
    let window = ns_window as *mut AnyObject;
    unsafe {
        if let Some(number) = behind_window_number.and_then(|number| isize::try_from(number).ok()) {
            let below = -1isize;
            let _: () = msg_send![window, orderWindow: below, relativeTo: number];
        } else {
            let _: () = msg_send![window, orderBack: std::ptr::null_mut::<AnyObject>()];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{managed_collection_behavior, NSWindowCollectionBehavior as Behavior};

    #[test]
    fn makes_default_spaces_behavior_explicit() {
        assert_eq!(
            managed_collection_behavior(Behavior::empty()),
            Behavior::Managed
        );
    }

    #[test]
    fn replaces_conflicting_spaces_behavior_and_preserves_other_flags() {
        let other_flags = Behavior::FullScreenAuxiliary | Behavior::IgnoresCycle;
        let expected = Behavior::Managed | other_flags;
        for behavior in [Behavior::Transient, Behavior::Stationary, Behavior::Managed] {
            assert_eq!(
                managed_collection_behavior(behavior | other_flags),
                expected
            );
        }
    }
}
