//! Prototype-only WindowServer compositing animation, driven by the display refresh clock.
use super::inbox_demo_freeze::PulseProbe;
use tauri::WebviewWindow;

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::NSRect;
    use std::{
        ffi::c_void,
        sync::{
            atomic::{AtomicU64, Ordering},
            Mutex, OnceLock,
        },
        time::{Duration, Instant},
    };
    use tokio::sync::oneshot;

    #[repr(C)]
    #[derive(Clone, Copy, Default, Debug)]
    struct Transform {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        tx: f64,
        ty: f64,
    }
    impl Transform {
        fn distance(self, other: Self) -> f64 {
            [
                self.a - other.a,
                self.b - other.b,
                self.c - other.c,
                self.d - other.d,
                self.tx - other.tx,
                self.ty - other.ty,
            ]
            .into_iter()
            .map(f64::abs)
            .fold(0.0, f64::max)
        }
        fn centered_scale(self, cx: f64, cy: f64, scale: f64) -> Self {
            // WindowServer maps global screen pixels into the backing buffer. Compose an
            // inverse scale around the global center with the captured original mapping.
            let inverse = 1.0 / scale;
            Self {
                a: self.a * inverse,
                b: self.b * inverse,
                c: self.c * inverse,
                d: self.d * inverse,
                tx: self.tx + (1.0 - inverse) * (self.a * cx + self.c * cy),
                ty: self.ty + (1.0 - inverse) * (self.b * cx + self.d * cy),
            }
        }
    }
    type MainConnection = unsafe extern "C" fn() -> i32;
    type GetTransform = unsafe extern "C" fn(i32, u32, *mut Transform) -> i32;
    type SetTransform = unsafe extern "C" fn(i32, u32, Transform) -> i32;
    type GetBounds = unsafe extern "C" fn(i32, u32, *mut NSRect) -> i32;
    struct Api {
        _handle: usize,
        connection: MainConnection,
        get: GetTransform,
        set: SetTransform,
        bounds: GetBounds,
    }
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    unsafe fn symbol(handle: *mut c_void, name: &std::ffi::CStr) -> Result<*mut c_void, String> {
        let ptr = libc::dlsym(handle, name.as_ptr());
        if ptr.is_null() {
            Err(format!(
                "SkyLight symbol {} is unavailable",
                name.to_string_lossy()
            ))
        } else {
            Ok(ptr)
        }
    }
    fn api() -> Result<&'static Api, String> {
        API.get_or_init(|| unsafe {
            let handle = libc::dlopen(
                c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            );
            if handle.is_null() {
                return Err("SkyLight could not be loaded".into());
            }
            // The system library remains loaded for the lifetime of these function pointers.
            Ok(Api {
                _handle: handle as usize,
                connection: std::mem::transmute::<*mut c_void, MainConnection>(symbol(
                    handle,
                    c"SLSMainConnectionID",
                )?),
                get: std::mem::transmute::<*mut c_void, GetTransform>(symbol(
                    handle,
                    c"SLSGetWindowTransform",
                )?),
                set: std::mem::transmute::<*mut c_void, SetTransform>(symbol(
                    handle,
                    c"SLSSetWindowTransform",
                )?),
                bounds: std::mem::transmute::<*mut c_void, GetBounds>(symbol(
                    handle,
                    c"SLSGetWindowBounds",
                )?),
            })
        })
        .as_ref()
        .map_err(Clone::clone)
    }

    type DisplayLink = *mut c_void;
    type Callback = unsafe extern "C" fn(
        DisplayLink,
        *const c_void,
        *const c_void,
        u64,
        *mut u64,
        *mut c_void,
    ) -> i32;
    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVDisplayLinkCreateWithCGDisplay(display: u32, output: *mut DisplayLink) -> i32;
        fn CVDisplayLinkSetOutputCallback(
            link: DisplayLink,
            callback: Callback,
            context: *mut c_void,
        ) -> i32;
        fn CVDisplayLinkStart(link: DisplayLink) -> i32;
        fn CVDisplayLinkStop(link: DisplayLink) -> i32;
        fn CVDisplayLinkRelease(link: DisplayLink);
    }

    struct Animation {
        id: u64,
        connection: i32,
        window: u32,
        original: Transform,
        cx: f64,
        cy: f64,
        started: Instant,
        last_frame: Option<Instant>,
        frames: u32,
        max_gap_ms: f64,
        peak_scale: f64,
        peak_readback: bool,
        done: bool,
        error: Option<String>,
    }
    static ACTIVE: OnceLock<Mutex<Option<Animation>>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(1);
    fn active() -> &'static Mutex<Option<Animation>> {
        ACTIVE.get_or_init(|| Mutex::new(None))
    }

    struct LinkGuard {
        handle: usize,
        id: u64,
    }
    impl Drop for LinkGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CVDisplayLinkStop(self.handle as DisplayLink);
                CVDisplayLinkRelease(self.handle as DisplayLink);
            }
            let mut slot = active().lock().unwrap();
            if let Some(anim) = slot.as_mut().filter(|a| a.id == self.id && !a.done) {
                if let Ok(api) = api() {
                    unsafe {
                        let _ = (api.set)(anim.connection, anim.window, anim.original);
                    }
                }
                anim.done = true;
            }
        }
    }

    const LIFT_DURATION: f64 = 0.115;
    const PULSE_DURATION: f64 = 0.54;
    fn scale_at(seconds: f64) -> f64 {
        // Each segment is monotonic, with zero velocity and acceleration at its endpoints.
        // The return never crosses the original scale, so the window edge cannot rebound.
        let smooth = |t: f64| {
            let t = t.clamp(0.0, 1.0);
            t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
        };
        let amount = if seconds <= LIFT_DURATION {
            smooth(seconds / LIFT_DURATION)
        } else {
            1.0 - smooth((seconds - LIFT_DURATION) / (PULSE_DURATION - LIFT_DURATION))
        };
        1.0 + 0.022 * amount
    }

    unsafe extern "C" fn frame_tick(
        _: DisplayLink,
        _: *const c_void,
        _: *const c_void,
        _: u64,
        _: *mut u64,
        context: *mut c_void,
    ) -> i32 {
        let mut slot = active().lock().unwrap();
        let id = context as usize as u64;
        let Some(anim) = slot.as_mut().filter(|a| a.id == id && !a.done) else {
            return 0;
        };
        let Ok(api) = api() else {
            anim.done = true;
            return 0;
        };
        let now = Instant::now();
        if let Some(previous) = anim.last_frame {
            anim.max_gap_ms = anim
                .max_gap_ms
                .max(now.duration_since(previous).as_secs_f64() * 1000.0);
        }
        anim.last_frame = Some(now);
        let elapsed = now.duration_since(anim.started).as_secs_f64();
        if elapsed >= PULSE_DURATION {
            let code = (api.set)(anim.connection, anim.window, anim.original);
            if code != 0 {
                anim.error = Some(format!("transform restoration failed: {code}"));
            }
            anim.done = true;
            return 0;
        }
        let scale = scale_at(elapsed);
        let transform = anim.original.centered_scale(anim.cx, anim.cy, scale);
        let code = (api.set)(anim.connection, anim.window, transform);
        if code != 0 {
            anim.error = Some(format!("WindowServer transform failed: {code}"));
            let _ = (api.set)(anim.connection, anim.window, anim.original);
            anim.done = true;
        } else {
            anim.frames += 1;
            if scale > anim.peak_scale {
                anim.peak_scale = scale;
                let mut readback = Transform::default();
                anim.peak_readback = (api.get)(anim.connection, anim.window, &mut readback) == 0
                    && transform.distance(readback) < 0.001;
            }
        }
        0
    }

    // Static callback storage stays alive even while a display link is being stopped. This
    // avoids a callback/context lifetime race during reset or cancellation.
    pub fn cancel() {
        let mut slot = active().lock().unwrap();
        if let Some(anim) = slot.as_mut().filter(|a| !a.done) {
            if let Ok(api) = api() {
                unsafe {
                    let _ = (api.set)(anim.connection, anim.window, anim.original);
                }
            }
            anim.done = true;
        }
    }

    pub async fn pulse(window: &WebviewWindow) -> Result<PulseProbe, String> {
        let api = api()?;
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let result = (|| {
                    let native = platform.ns_window() as *mut AnyObject;
                    let frame: NSRect = msg_send![native, frame];
                    let visible: bool = msg_send![native, isVisible];
                    let workspace: *mut AnyObject =
                        msg_send![AnyClass::get(c"NSWorkspace").unwrap(), sharedWorkspace];
                    let reduced: bool =
                        msg_send![workspace, accessibilityDisplayShouldReduceMotion];
                    let wid: i64 = msg_send![native, windowNumber];
                    let connection = (api.connection)();
                    let mut original = Transform::default();
                    let mut bounds = NSRect::default();
                    let code = (api.get)(connection, wid as u32, &mut original);
                    if code != 0 {
                        return Err(format!(
                            "get original WindowServer transform failed: {code}"
                        ));
                    }
                    let code = (api.bounds)(connection, wid as u32, &mut bounds);
                    if code != 0 {
                        return Err(format!("get WindowServer bounds failed: {code}"));
                    }
                    let screen: *mut AnyObject = msg_send![native, screen];
                    let description: *mut AnyObject = msg_send![screen, deviceDescription];
                    let key = objc2_foundation::NSString::from_str("NSScreenNumber");
                    let number: *mut AnyObject = msg_send![description, objectForKey: &*key];
                    let display: u32 = msg_send![number, unsignedIntValue];
                    Ok((
                        wid as u32,
                        connection,
                        original,
                        bounds,
                        frame,
                        display,
                        visible && !reduced,
                    ))
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        let (wid, connection, original, bounds, native_frame, display, animate) = rx
            .await
            .map_err(|_| "private animation preparation disconnected".to_string())??;
        let mut probe = PulseProbe {
            original_width: native_frame.size.width,
            peak_width: native_frame.size.width,
            final_width: native_frame.size.width,
            original_x: native_frame.origin.x,
            final_x: native_frame.origin.x,
            animated: animate,
            frame_stable: true,
            transform_restored: true,
            frames: 0,
            peak_scale: 1.0,
            max_gap_ms: 0.0,
            peak_readback: false,
        };
        if !animate {
            return Ok(probe);
        }
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        cancel();
        *active().lock().unwrap() = Some(Animation {
            id,
            connection,
            window: wid,
            original,
            cx: bounds.origin.x + bounds.size.width / 2.0,
            cy: bounds.origin.y + bounds.size.height / 2.0,
            started: Instant::now(),
            last_frame: None,
            frames: 0,
            max_gap_ms: 0.0,
            peak_scale: 1.0,
            peak_readback: false,
            done: false,
            error: None,
        });
        let link = {
            let mut handle: DisplayLink = std::ptr::null_mut();
            let code = unsafe { CVDisplayLinkCreateWithCGDisplay(display, &mut handle) };
            if code != 0 {
                cancel();
                return Err(format!("display link creation failed: {code}"));
            }
            let guard = LinkGuard {
                handle: handle as usize,
                id,
            };
            let code = unsafe {
                CVDisplayLinkSetOutputCallback(handle, frame_tick, id as usize as *mut c_void)
            };
            if code != 0 {
                return Err(format!("display callback setup failed: {code}"));
            }
            let code = unsafe { CVDisplayLinkStart(handle) };
            if code != 0 {
                return Err(format!("display link start failed: {code}"));
            }
            guard
        };
        // The async command owns only an integer handle; all per-frame work runs on the native
        // display clock, without JS IPC, AppKit resize messages, or WebView snapshots.
        for _ in 0..125 {
            if active()
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|a| a.id != id || a.done)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
        drop(link);
        let anim = {
            let mut slot = active().lock().unwrap();
            if slot.as_ref().is_none_or(|a| a.id != id) {
                return Err("animation was replaced".into());
            }
            slot.take().unwrap()
        };
        unsafe {
            let _ = (api.set)(connection, wid, original);
        }
        probe.frames = anim.frames;
        probe.peak_scale = anim.peak_scale;
        probe.peak_width = native_frame.size.width * anim.peak_scale;
        probe.max_gap_ms = anim.max_gap_ms;
        probe.peak_readback = anim.peak_readback;
        let mut restored = Transform::default();
        probe.transform_restored = unsafe { (api.get)(connection, wid, &mut restored) == 0 }
            && original.distance(restored) < 0.001;
        if let Some(error) = anim.error {
            return Err(error);
        }
        if !anim.done {
            return Err("display animation timed out; original transform restored".into());
        }
        let (tx, rx) = oneshot::channel();
        window
            .with_webview(move |platform| unsafe {
                let native = platform.ns_window() as *mut AnyObject;
                let final_frame: NSRect = msg_send![native, frame];
                let _ = tx.send(final_frame);
            })
            .map_err(|e| e.to_string())?;
        let final_frame = rx
            .await
            .map_err(|_| "animation frame verification disconnected".to_string())?;
        probe.final_width = final_frame.size.width;
        probe.final_x = final_frame.origin.x;
        probe.frame_stable = final_frame == native_frame;
        Ok(probe)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn compositing_scale_preserves_window_center() {
            let original = Transform {
                a: 1.0,
                d: 1.0,
                tx: -470.0,
                ty: -100.0,
                ..Transform::default()
            };
            let (cx, cy) = (750.0, 426.0);
            let scaled = original.centered_scale(cx, cy, 1.022);
            assert!((scaled.a * cx + scaled.c * cy + scaled.tx - 280.0).abs() < 1e-9);
            assert!((scaled.b * cx + scaled.d * cy + scaled.ty - 326.0).abs() < 1e-9);
            assert!(original.distance(original.centered_scale(cx, cy, 1.0)) < 1e-9);
        }
        #[test]
        fn pulse_curve_returns_without_rebounding() {
            let mut previous = scale_at(LIFT_DURATION);
            for i in 1..=1000 {
                let t = LIFT_DURATION + (PULSE_DURATION - LIFT_DURATION) * i as f64 / 1000.0;
                let scale = scale_at(t);
                assert!((1.0..=1.022).contains(&scale));
                assert!(scale <= previous);
                previous = scale;
            }
            assert_eq!(scale_at(PULSE_DURATION), 1.0);
            assert_eq!(scale_at(PULSE_DURATION + 1.0), 1.0);
        }
        #[test]
        fn pulse_curve_has_smooth_peak_and_end() {
            let dt = 0.00001;
            let before = (scale_at(LIFT_DURATION) - scale_at(LIFT_DURATION - dt)) / dt;
            let after = (scale_at(LIFT_DURATION + dt) - scale_at(LIFT_DURATION)) / dt;
            assert!(before.abs() < 0.001 && after.abs() < 0.001);
            let ending = (scale_at(PULSE_DURATION) - scale_at(PULSE_DURATION - dt)) / dt;
            assert!(ending.abs() < 0.001);
            let acceleration = (scale_at(PULSE_DURATION) - 2.0 * scale_at(PULSE_DURATION - dt)
                + scale_at(PULSE_DURATION - 2.0 * dt))
                / (dt * dt);
            assert!(acceleration.abs() < 0.001);
        }
    }
}

pub async fn pulse(window: &WebviewWindow) -> Result<PulseProbe, String> {
    #[cfg(target_os = "macos")]
    {
        mac::pulse(window).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err("WindowServer transform prototype requires macOS".into())
    }
}

pub fn cancel() {
    #[cfg(target_os = "macos")]
    mac::cancel();
}
