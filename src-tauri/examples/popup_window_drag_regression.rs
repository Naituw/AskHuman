//! Exercise the patched Tao through Tauri, without daemon traffic or real pointer input.
//! Native input state and AppKit's final drag call are replaced only inside this test process.
#[cfg(target_os = "macos")]
mod regression {
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, Bool, Imp, Method, Sel},
        sel,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

    static BUTTONS: AtomicUsize = AtomicUsize::new(1);
    thread_local! {
        static CAPTURED: RefCell<Vec<Retained<AnyObject>>> = const { RefCell::new(Vec::new()) };
        static CURRENT: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    unsafe extern "C" fn record_drag(_: &AnyObject, _: Sel, event: *mut AnyObject) {
        CAPTURED.with(|slot| slot.borrow_mut().push(Retained::retain(event).unwrap()));
    }
    unsafe extern "C" fn pressed(_: &AnyObject, _: Sel) -> usize {
        BUTTONS.load(Ordering::SeqCst)
    }
    unsafe extern "C" fn current(_: &AnyObject, _: Sel) -> *mut AnyObject {
        CURRENT.with(|slot| {
            slot.borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |e| Retained::as_ptr(e).cast_mut())
        })
    }

    pub(super) struct Hooks(Vec<(&'static Method, Imp)>);
    impl Hooks {
        unsafe fn replace(&mut self, method: &'static Method, implementation: Imp) {
            self.0
                .push((method, method.set_implementation(implementation)));
        }
    }
    impl Drop for Hooks {
        fn drop(&mut self) {
            for (method, original) in self.0.drain(..).rev() {
                unsafe { method.set_implementation(original) };
            }
        }
    }

    pub(super) unsafe fn forbid_activation() -> Hooks {
        unsafe extern "C" fn ignore(_: &AnyObject, _: Sel, _: Bool) {}
        let mut hooks = Hooks(Vec::new());
        hooks.replace(
            AnyClass::get(c"NSApplication")
                .unwrap()
                .instance_method(sel!(activateIgnoringOtherApps:))
                .unwrap(),
            std::mem::transmute::<unsafe extern "C" fn(&AnyObject, Sel, Bool), Imp>(ignore),
        );
        hooks
    }

    unsafe fn native(window: &WebviewWindow) -> &AnyObject {
        &*window.ns_window().unwrap().cast::<AnyObject>()
    }
    unsafe fn event(window: &WebviewWindow, kind: usize, point: NSPoint) -> Retained<AnyObject> {
        let number: isize = msg_send![native(window), windowNumber];
        msg_send![AnyClass::get(c"NSEvent").unwrap(),
            mouseEventWithType: kind, location: point, modifierFlags: 0usize,
            timestamp: 1.0f64, windowNumber: number, context: std::ptr::null::<AnyObject>(),
            eventNumber: 123isize, clickCount: 1isize, pressure: 1.0f32]
    }
    unsafe fn send(window: &WebviewWindow, kind: usize, point: NSPoint) -> Retained<AnyObject> {
        let event = event(window, kind, point);
        let _: () = msg_send![native(window), sendEvent: &*event];
        event
    }
    fn take() -> Vec<Retained<AnyObject>> {
        CAPTURED.with(|slot| std::mem::take(&mut *slot.borrow_mut()))
    }
    fn expect_none(window: &WebviewWindow) {
        window.start_dragging().unwrap();
        assert!(
            take().is_empty(),
            "an ineligible request started a native drag"
        );
    }

    pub unsafe fn run(app: &tauri::App) {
        let make = |label| {
            WebviewWindowBuilder::new(
                app,
                label,
                WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .visible(false)
            .focused(false)
            .inner_size(480.0, 420.0)
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .build()
            .unwrap()
        };
        let a = make("drag-regression-a");
        let b = make("drag-regression-b");
        let mut hooks = Hooks(Vec::new());
        hooks.replace(
            AnyClass::get(c"NSWindow")
                .unwrap()
                .instance_method(sel!(performWindowDragWithEvent:))
                .unwrap(),
            std::mem::transmute::<unsafe extern "C" fn(&AnyObject, Sel, *mut AnyObject), Imp>(
                record_drag,
            ),
        );
        hooks.replace(
            AnyClass::get(c"NSEvent")
                .unwrap()
                .class_method(sel!(pressedMouseButtons))
                .unwrap(),
            std::mem::transmute::<unsafe extern "C" fn(&AnyObject, Sel) -> usize, Imp>(pressed),
        );
        hooks.replace(
            AnyClass::get(c"NSApplication")
                .unwrap()
                .instance_method(sel!(currentEvent))
                .unwrap(),
            std::mem::transmute::<unsafe extern "C" fn(&AnyObject, Sel) -> *mut AnyObject, Imp>(
                current,
            ),
        );

        let mut count = 0;
        for (x, y) in [
            (-2000.0, 200.0),
            (400.0, 200.0),
            (2400.0, 200.0),
            (200.0, 1400.0),
            (-1500.0, -900.0),
            (0.0, 0.0),
        ] {
            let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(480.0, 420.0));
            let _: () = msg_send![native(&a), setFrame: frame, display: false];
            for point in [
                NSPoint::new(120.0, 210.0),
                NSPoint::new(12.0, 200.0),
                NSPoint::new(400.0, 190.0),
            ] {
                for stale in [true, false] {
                    let original = send(&a, 1, point);
                    let identity = Retained::as_ptr(&original);
                    let screen: NSPoint = msg_send![native(&a), convertPointToScreen: point];
                    // A window-associated screen point is the corrupt result of Tao's old fallback.
                    CURRENT.with(|slot| *slot.borrow_mut() = stale.then(|| event(&a, 1, screen)));
                    drop(original); // The production cache must own the original across IPC.
                    a.start_dragging().unwrap();
                    let captured = take();
                    assert_eq!(captured.len(), 1, "the original event was lost across IPC");
                    assert_eq!(
                        Retained::as_ptr(&captured[0]),
                        identity,
                        "used the current event instead of the original"
                    );
                    let actual: NSPoint = msg_send![&*captured[0], locationInWindow];
                    assert_eq!(actual, point);
                    let unchanged: NSRect = msg_send![native(&a), frame];
                    assert_eq!(unchanged, frame);
                    expect_none(&a); // A second request cannot consume the same click again.
                    count += 1;
                }
            }
        }

        let p = NSPoint::new(100.0, 200.0);
        send(&a, 1, p);
        expect_none(&b); // Rejecting another window must preserve the rightful owner's event.
        a.start_dragging().unwrap();
        assert_eq!(take().len(), 1);

        send(&a, 1, p);
        send(&b, 1, p);
        expect_none(&a); // A newer primary gesture supersedes the old window's event.
        b.start_dragging().unwrap();
        assert_eq!(take().len(), 1);

        send(&a, 1, p);
        send(&a, 2, p);
        expect_none(&a); // Mouse-up clears even if a later click makes the primary button down again.
        send(&a, 3, p);
        expect_none(&a); // Right mouse-down is not an eligible primary gesture.

        send(&a, 1, p);
        BUTTONS.store(0, Ordering::SeqCst);
        expect_none(&a); // AppKit may consume mouse-up inside native drag tracking.
        BUTTONS.store(1, Ordering::SeqCst);
        expect_none(&a); // The released gesture was discarded, not merely postponed.

        send(&a, 1, p);
        let _: () = msg_send![native(&a), close];
        expect_none(&a);
        println!("PASS: {count} native drag cases plus duplicate, window ownership, mouse-up, released-button and close checks");
        CURRENT.with(|slot| slot.borrow_mut().take());
        drop(hooks);
    }
}

#[cfg(target_os = "macos")]
fn main() {
    let _activation_guard = unsafe { regression::forbid_activation() };
    tauri::Builder::default()
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            unsafe { regression::run(app) };
            app.handle().exit(0);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("native drag regression failed");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Native window dragging regression requires macOS");
}
