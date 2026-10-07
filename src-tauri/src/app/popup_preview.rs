//! Native geometry owner for the fixed right-side Popup attachment panel.
use super::popup_preview_geometry::{self as geometry, Rect, Side, DIVIDER};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize, Window};
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub revision: u64,
    pub side: Side,
    pub main_width: f64,
    pub main_height: f64,
}
pub struct SizeProjection {
    pub main: (u32, u32),
    pub preview: Option<f64>,
}
#[derive(Default)]
pub struct Controller {
    pub(super) inbox: super::popup_inbox_geometry::Geometry,
    side: Side,
    main: (f64, f64),
    preferred_preview: Option<f64>,
    preview_baseline: Option<Target>,
    inside_fraction: Option<f64>,
    special_width: Option<f64>,
    origin: Option<Target>,
    user_moved: bool,
    native_width_changed: bool,
    pending: bool,
    intent_version: u64,
    revision: u64,
    published: Option<Layout>,
    normal: Option<Target>,
    restore_on_normal: Option<Target>,
    prepared: Option<Preparation>,
}
struct Native {
    outer: Rect,
    inner: (f64, f64),
    scale: f64,
    positioning: bool,
    special: bool,
}
#[derive(Clone, Copy)]
struct Target {
    outer: Rect,
    inner: (f64, f64),
    scale: f64,
    positioning: bool,
}
struct Preparation {
    frame: Target,
    version: u64,
    open: bool,
    owner_side: Side,
}
impl Target {
    fn from_native(n: &Native) -> Self {
        Self {
            outer: n.outer,
            inner: (n.inner.0 * n.scale, n.inner.1 * n.scale),
            scale: n.scale,
            positioning: n.positioning,
        }
    }
    fn factor(self, scale: f64) -> f64 {
        #[cfg(target_os = "macos")]
        {
            scale / self.scale
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = scale;
            1.0
        }
    }
    fn position_matches(self, actual: &Native) -> bool {
        let factor = self.factor(actual.scale);
        !self.positioning
            || ((actual.outer.x - self.outer.x * factor).abs() <= 2.0
                && (actual.outer.y - self.outer.y * factor).abs() <= 2.0)
    }
    fn matches(self, actual: &Native) -> bool {
        let factor = self.factor(actual.scale);
        self.position_matches(actual)
            && (actual.inner.0 * actual.scale - self.inner.0 * factor).abs() <= 2.0
            && (actual.inner.1 * actual.scale - self.inner.1 * factor).abs() <= 2.0
    }
    fn apply(self, window: &Window) -> Result<(), String> {
        let factor = self.factor(window.scale_factor().map_err(|e| e.to_string())?);
        window
            .set_size(PhysicalSize::new(
                (self.inner.0 * factor).round().max(1.0) as u32,
                (self.inner.1 * factor).round().max(1.0) as u32,
            ))
            .map_err(|e| e.to_string())?;
        if self.positioning {
            window
                .set_position(PhysicalPosition::new(
                    (self.outer.x * factor).round() as i32,
                    (self.outer.y * factor).round() as i32,
                ))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
pub fn request(window: &Window, request_id: &str) -> Result<crate::models::AskRequest, String> {
    assigned_request(window, request_id, true)
}
pub fn assigned_request(
    window: &Window,
    request_id: &str,
    require_presented: bool,
) -> Result<crate::models::AskRequest, String> {
    if window.label() != "popup" {
        return Err("attachment preview requires a Popup".into());
    }
    let app = window.app_handle();
    if let Some(inbox) = app.try_state::<super::popup_inbox::Inbox>() {
        if require_presented && !inbox.active(request_id) {
            return Err("Popup request is not active".into());
        }
        return inbox
            .request(request_id)?
            .interaction
            .ask()
            .cloned()
            .ok_or("attachment preview requires a question".into());
    }
    let bridge = app
        .try_state::<super::GuiBridge>()
        .ok_or("Popup is not assigned")?;
    if bridge.is_done()
        || (require_presented && !bridge.presented.load(std::sync::atomic::Ordering::SeqCst))
    {
        return Err("Popup is not active".into());
    }
    let request = if let Some(warm) = app.try_state::<super::WarmPopup>() {
        warm.show
            .lock()
            .map_err(|_| "Popup unavailable")?
            .as_ref()
            .and_then(|show| show.interaction.ask())
            .cloned()
    } else {
        app.state::<super::AppState>().interaction.ask().cloned()
    }
    .ok_or("attachment preview requires a question")?;
    if request.id != request_id {
        return Err("Popup request changed".into());
    }
    Ok(request)
}

fn sample(window: &Window) -> Result<Native, String> {
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let outer = window.outer_size().map_err(|e| e.to_string())?;
    let position = window.outer_position().ok();
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    Ok(Native {
        outer: Rect {
            x: position.map_or(0.0, |p| p.x as f64),
            y: position.map_or(0.0, |p| p.y as f64),
            width: outer.width as f64,
            height: outer.height as f64,
        },
        inner: (inner.width as f64 / scale, inner.height as f64 / scale),
        scale,
        positioning: position.is_some() && can_position(window),
        special: window.is_maximized().unwrap_or(true) || window.is_fullscreen().unwrap_or(true),
    })
}
#[cfg(not(target_os = "linux"))]
fn can_position(_: &Window) -> bool {
    true
}
#[cfg(target_os = "linux")]
fn can_position(window: &Window) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window_handle()
        .is_ok_and(|h| !matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
}
fn work_for(window: &Window, main: Rect, scale: f64) -> Option<Rect> {
    window
        .available_monitors()
        .ok()?
        .into_iter()
        .map(|m| {
            let work = m.work_area();
            #[cfg(target_os = "macos")]
            let factor = scale / m.scale_factor();
            #[cfg(not(target_os = "macos"))]
            let factor = {
                let _ = scale;
                1.0
            };
            Rect {
                x: work.position.x as f64 * factor,
                y: work.position.y as f64 * factor,
                width: work.size.width as f64 * factor,
                height: work.size.height as f64 * factor,
            }
        })
        .max_by(|a, b| main.intersection(*a).total_cmp(&main.intersection(*b)))
}
impl Controller {
    fn layout(&self, n: &Native) -> Layout {
        if self.pending {
            return self.published.clone().unwrap_or(Layout {
                revision: self.revision,
                side: Side::Closed,
                main_width: n.inner.0,
                main_height: n.inner.1,
            });
        }
        let side = if n.special && self.side != Side::Closed {
            Side::Inside
        } else {
            self.side
        };
        let width = if side == Side::Closed {
            n.inner.0
        } else {
            geometry::inside_main_width(
                n.inner.0,
                self.special_width
                    .filter(|_| n.special)
                    .unwrap_or(self.main.0),
                if side == Side::Inside {
                    self.inside_fraction
                } else {
                    None
                },
            )
        };
        Layout {
            revision: self.revision,
            side,
            main_width: width,
            main_height: n.inner.1,
        }
    }
    fn publish(&mut self, window: &Window, n: &Native) -> Layout {
        if !n.special {
            self.normal = Some(Target::from_native(n));
        }
        self.revision += 1;
        let layout = self.layout(n);
        self.published = Some(layout.clone());
        let _ = window.emit("popup-preview-layout", &layout);
        layout
    }
    fn project(&mut self, n: &Native) {
        if self.side == Side::Closed || n.special {
            return;
        }
        self.main.1 = n.inner.1;
        self.special_width = None;
        self.native_width_changed = (n.inner.0 - self.main.0).abs() > 1.0 / n.scale;
        if n.inner.0 < self.main.0 + 326.0 || !n.positioning {
            self.side = Side::Inside;
        } else if self.inside_fraction.is_none() {
            self.side = Side::Right;
        }
    }
    fn notice_move(&mut self, n: &Native) {
        if self.side != Side::Closed
            && !n.special
            && self.normal.is_some_and(|old| !old.position_matches(n))
        {
            self.user_moved = true;
        }
    }
    fn remember_preview(&mut self, n: &Native, work_constrained: bool) -> Option<f64> {
        let previous = self.preview_baseline?;
        if self.pending
            || self.side != Side::Right
            || n.special
            || work_constrained
            || (previous.scale - n.scale).abs() > f64::EPSILON
            || (previous.inner.0 / previous.scale - n.inner.0).abs() <= 1.0 / n.scale
        {
            return None;
        }
        let width = n.inner.0 - self.main.0 - DIVIDER;
        if width < 320.0 || !width.is_finite() {
            return None;
        }
        self.preferred_preview = Some(width);
        Some(width)
    }
    fn close_target(&self, n: &Native) -> Target {
        let mut target = Target::from_native(n);
        target.inner = (self.main.0 * n.scale, self.main.1 * n.scale);
        target.outer.width += target.inner.0 - n.inner.0 * n.scale;
        target.outer.height += target.inner.1 - n.inner.1 * n.scale;
        if !self.user_moved {
            if let Some(origin) = self.origin {
                let factor = origin.factor(n.scale);
                target.outer.x = origin.outer.x * factor;
                target.outer.y = origin.outer.y * factor;
            }
        }
        target
    }
}
pub fn change(
    window: &Window,
    open: bool,
    main_extent: Option<f64>,
    version: u64,
) -> Result<Layout, String> {
    if main_extent.is_none() {
        let n = sample(window)?;
        let state = window.app_handle().state::<Mutex<Controller>>();
        let mut c = state.lock().map_err(|_| "preview layout unavailable")?;
        let prepared = c.prepared.take().ok_or("preview geometry changed")?;
        if prepared.version != version
            || prepared.open != open
            || prepared.owner_side != c.side
            || !prepared.frame.matches(&n)
        {
            return Err("preview geometry changed".into());
        }
    }
    change_impl(window, open, main_extent, version)
}
/// Freeze the visible main region before native setters enqueue their work.
pub fn prepare(window: &Window, open: bool, version: u64) -> Result<Layout, String> {
    let n = sample(window)?;
    let state = window.app_handle().state::<Mutex<Controller>>();
    let mut c = state.lock().map_err(|_| "preview layout unavailable")?;
    if c.pending || version < c.intent_version {
        return Err("preview geometry changed".into());
    }
    let mut layout = c.layout(&n);
    if open && c.side == Side::Closed {
        layout.side = Side::Right;
    }
    c.prepared = Some(Preparation {
        frame: Target::from_native(&n),
        version,
        open,
        owner_side: c.side,
    });
    Ok(layout)
}
pub async fn wait_idle(window: &Window) -> Result<(), String> {
    if window
        .app_handle()
        .try_state::<super::popup_inbox::Inbox>()
        .is_some()
    {
        return super::popup_inbox_geometry::wait_idle(window).await;
    }
    loop {
        if window
            .app_handle()
            .try_state::<super::GuiBridge>()
            .is_none_or(|b| b.is_done())
        {
            return Err("Popup is no longer active".into());
        }
        if !window
            .app_handle()
            .state::<Mutex<Controller>>()
            .lock()
            .map_err(|_| "preview layout unavailable")?
            .pending
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
pub fn published(window: &Window) -> Option<Layout> {
    if window
        .app_handle()
        .try_state::<super::popup_inbox::Inbox>()
        .is_some()
    {
        return super::popup_inbox_geometry::published(window);
    }
    window
        .app_handle()
        .state::<Mutex<Controller>>()
        .lock()
        .ok()?
        .published
        .clone()
}
fn change_impl(
    window: &Window,
    open: bool,
    main_extent: Option<f64>,
    version: u64,
) -> Result<Layout, String> {
    let n = sample(window)?;
    let state = window.app_handle().state::<Mutex<Controller>>();
    let mut c = state.lock().map_err(|_| "preview layout unavailable")?;
    if version < c.intent_version {
        return Ok(c.layout(&n));
    }
    if c.pending {
        return Err("preview layout is still changing".into());
    }
    c.intent_version = version;
    let was_open = c.side != Side::Closed;
    if open == was_open && main_extent.is_none() {
        return Ok(c.layout(&n));
    }
    if !was_open {
        c.preview_baseline = Some(Target::from_native(&n));
        let config = crate::config::AppConfig::load_without_secrets()
            .channels
            .popup;
        if c.preferred_preview.is_none() || !config.remember_size {
            c.preferred_preview = Some(super::popup_size::restored_preview_width(&config));
        }
        c.main = if n.special {
            c.normal
                .map(|frame| (frame.inner.0 / frame.scale, frame.inner.1 / frame.scale))
                .unwrap_or_else(|| super::popup_size::restored_size(&config))
        } else {
            n.inner
        };
        c.origin = if n.special {
            c.normal
        } else {
            Some(Target::from_native(&n))
        };
        c.user_moved = false;
        c.native_width_changed = false;
        c.inside_fraction = None;
        c.special_width = n.special.then_some(n.inner.0);
    }
    if let Some(extent) = main_extent.filter(|n| n.is_finite()) {
        let durable = c.side == Side::Right && !n.special && n.inner.0 >= 746.0;
        if durable {
            c.main.0 = extent.clamp(420.0, n.inner.0 - 326.0);
            c.preferred_preview = Some(n.inner.0 - c.main.0 - DIVIDER);
        } else {
            c.inside_fraction = Some(extent / (n.inner.0 - DIVIDER).max(1.0));
        }
        c.project(&n);
        let layout = c.publish(window, &n);
        let main = c.main;
        let preview = c.preferred_preview;
        drop(c);
        remember_baseline(window, main);
        if durable {
            save_dimensions(main, preview);
        }
        return Ok(layout);
    }
    if n.special {
        if !open && c.native_width_changed {
            c.restore_on_normal = c.normal.map(|frame| {
                let normal = Native {
                    outer: frame.outer,
                    inner: (frame.inner.0 / frame.scale, frame.inner.1 / frame.scale),
                    scale: frame.scale,
                    positioning: frame.positioning,
                    special: false,
                };
                c.close_target(&normal)
            });
        }
        c.side = if open { Side::Inside } else { Side::Closed };
        return Ok(c.publish(window, &n));
    }
    c.notice_move(&n);
    let fallback = if open {
        Target::from_native(&n)
    } else {
        c.close_target(&n)
    };
    let target = if open && n.positioning {
        let decoration = (
            n.outer.width - n.inner.0 * n.scale,
            n.outer.height - n.inner.1 * n.scale,
        );
        let main = Rect {
            width: c.main.0 * n.scale + decoration.0,
            height: c.main.1 * n.scale + decoration.1,
            ..n.outer
        };
        let (side, outer) = work_for(window, main, n.scale)
            .map_or((Side::Inside, n.outer), |work| {
                geometry::place_right(main, work, n.scale, c.preferred_preview.unwrap_or(700.0))
            });
        c.side = side;
        Target {
            outer,
            inner: (outer.width - decoration.0, outer.height - decoration.1),
            scale: n.scale,
            positioning: true,
        }
    } else {
        c.side = if open { Side::Inside } else { Side::Closed };
        fallback
    };
    c.pending = true;
    let layout = c.layout(&n);
    drop(c);
    if target.apply(window).is_err() {
        state.lock().unwrap().side = if open { Side::Inside } else { Side::Closed };
        let _ = fallback.apply(window);
        settle(window, fallback, None, version);
    } else {
        settle(window, target, open.then_some(fallback), version);
    }
    Ok(layout)
}
// A successful setter is not an acknowledgement on macOS. Publish only after the frame settles.
fn settle(window: &Window, initial: Target, rollback: Option<Target>, version: u64) {
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        let mut target = initial;
        let mut fallback = rollback;
        let mut attempts = 0;
        let mut interrupted = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            let w = window.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            if window
                .app_handle()
                .run_on_main_thread(move || {
                    let state = w.app_handle().state::<Mutex<Controller>>();
                    let mut c = state.lock().unwrap();
                    if c.intent_version != version
                        || !c.pending
                        || w.app_handle()
                            .try_state::<super::GuiBridge>()
                            .is_some_and(|b| b.is_done())
                    {
                        let _ = tx.send((false, false, None));
                        return;
                    }
                    let Ok(n) = sample(&w) else {
                        c.pending = false;
                        let _ = tx.send((false, false, None));
                        return;
                    };
                    if primary_button_down(&w) {
                        let _ = tx.send((true, true, None));
                        return;
                    }
                    if target.matches(&n) || interrupted || (attempts >= 20 && fallback.is_none()) {
                        c.pending = false;
                        if interrupted && !target.position_matches(&n) {
                            c.user_moved = true;
                        }
                        c.project(&n);
                        c.preview_baseline = Some(Target::from_native(&n));
                        remember_baseline(
                            &w,
                            if c.side == Side::Closed {
                                n.inner
                            } else {
                                c.main
                            },
                        );
                        c.publish(&w, &n);
                        let _ = tx.send((false, false, None));
                    } else if attempts >= 20 {
                        let backup = fallback.unwrap();
                        c.side = Side::Inside;
                        drop(c);
                        let _ = backup.apply(&w);
                        let _ = tx.send((true, false, Some(backup)));
                    } else {
                        let _ = tx.send((true, false, None));
                    }
                })
                .is_err()
            {
                break;
            }
            let Ok((again, held, backup)) = rx.await else {
                break;
            };
            if !again {
                break;
            }
            if held {
                interrupted = true;
            } else {
                attempts += 1;
            }
            if let Some(backup) = backup {
                target = backup;
                fallback = None;
                attempts = 0;
            }
        }
    });
}
fn remember_baseline(window: &Window, main: (f64, f64)) {
    window
        .app_handle()
        .state::<Mutex<super::popup_size::SizeMemory>>()
        .lock()
        .unwrap()
        .restoring(main);
}
pub(super) fn save_dimensions(main: (f64, f64), preview: Option<f64>) {
    if main.0 < 420.0 || main.1 < 480.0 {
        return;
    }
    // Save before the layout command returns, including when the user immediately submits.
    let mut cfg = crate::config::AppConfig::load_without_secrets();
    if cfg.channels.popup.remember_size {
        cfg.channels.popup.width = main.0;
        cfg.channels.popup.height = main.1;
        if let Some(width) = preview {
            cfg.channels.popup.preview_width = width;
        }
        let _ = cfg.save();
    }
}
pub fn resized(window: &Window, event: PhysicalSize<u32>) -> Option<SizeProjection> {
    if window
        .app_handle()
        .try_state::<super::popup_inbox::Inbox>()
        .is_some()
    {
        return super::popup_inbox_geometry::resized(window, event);
    }
    let n = sample(window).ok()?;
    if window.inner_size().ok()? != event {
        return None;
    }
    let state = window.app_handle().state::<Mutex<Controller>>();
    let mut c = state.lock().ok()?;
    if c.pending {
        return None;
    }
    if n.special {
        c.publish(window, &n);
        return None;
    }
    if let Some(target) = c.restore_on_normal.take() {
        c.pending = true;
        let version = c.intent_version;
        drop(c);
        let w = window.clone();
        let _ = window.app_handle().run_on_main_thread(move || {
            let _ = target.apply(&w);
            settle(&w, target, None, version);
        });
        return None;
    }
    c.notice_move(&n);
    if c.side == Side::Closed {
        c.normal = Some(Target::from_native(&n));
        c.preview_baseline = c.normal;
        return Some(SizeProjection {
            main: (event.width, event.height),
            preview: None,
        });
    }
    c.project(&n);
    // A monitor/work-area restriction is temporary, just like an opening transaction.
    let constrained = work_for(window, n.outer, n.scale).is_some_and(|work| {
        let decoration = n.outer.width - n.inner.0 * n.scale;
        let wanted =
            (c.main.0 + c.preferred_preview.unwrap_or(700.0) + DIVIDER) * n.scale + decoration;
        wanted > work.width + 2.0 && n.outer.width >= work.width - 2.0
    });
    let preview = c.remember_preview(&n, constrained);
    // Moving the left edge may emit Moved before Resized. Only resize acknowledgements update
    // this baseline, so the move callback cannot consume the user's width change.
    c.preview_baseline = Some(Target::from_native(&n));
    c.publish(window, &n);
    let projected = (
        (c.main.0 * n.scale).round() as u32,
        (c.main.1 * n.scale).round() as u32,
    );
    Some(SizeProjection {
        main: projected,
        preview,
    })
}
pub fn moved(window: &Window) {
    if window
        .app_handle()
        .try_state::<super::popup_inbox::Inbox>()
        .is_some()
    {
        super::popup_inbox_geometry::moved(window);
        return;
    }
    let Ok(n) = sample(window) else {
        return;
    };
    let state = window.app_handle().state::<Mutex<Controller>>();
    let mut c = state.lock().unwrap();
    if c.pending {
        return;
    }
    c.notice_move(&n);
    c.publish(window, &n);
}
#[cfg(target_os = "macos")]
pub(super) fn primary_button_down(_: &Window) -> bool {
    objc2_app_kit::NSEvent::pressedMouseButtons() & 1 != 0
}
#[cfg(target_os = "windows")]
pub(super) fn primary_button_down(_: &Window) -> bool {
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(1) < 0 }
}
#[cfg(target_os = "linux")]
pub(super) fn primary_button_down(window: &Window) -> bool {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    #[link(name = "X11")]
    unsafe extern "C" {
        fn XQueryPointer(
            display: *mut std::ffi::c_void,
            window: std::os::raw::c_ulong,
            root: *mut std::os::raw::c_ulong,
            child: *mut std::os::raw::c_ulong,
            rx: *mut i32,
            ry: *mut i32,
            wx: *mut i32,
            wy: *mut i32,
            mask: *mut u32,
        ) -> i32;
    }
    if let (Ok(display), Ok(handle)) = (window.display_handle(), window.window_handle()) {
        if let (RawDisplayHandle::Xlib(d), RawWindowHandle::Xlib(w)) =
            (display.as_raw(), handle.as_raw())
        {
            if let Some(d) = d.display {
                let (mut root, mut child, mut rx, mut ry, mut wx, mut wy, mut mask) =
                    (0, 0, 0, 0, 0, 0, 0);
                unsafe {
                    XQueryPointer(
                        d.as_ptr(),
                        w.window,
                        &mut root,
                        &mut child,
                        &mut rx,
                        &mut ry,
                        &mut wx,
                        &mut wy,
                        &mut mask,
                    );
                }
                return mask & (1 << 8) != 0;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn native(width: f64) -> Native {
        Native {
            outer: Rect {
                x: 100.0,
                y: 100.0,
                width,
                height: 652.0,
            },
            inner: (width, 620.0),
            scale: 1.0,
            positioning: true,
            special: false,
        }
    }
    #[test]
    fn queued_native_resize_does_not_acknowledge_or_publish_early() {
        let target = Target::from_native(&native(1126.0));
        assert!(!target.matches(&native(560.0)));
        assert!(target.matches(&native(1126.0)));
        let c = Controller {
            side: Side::Right,
            main: (560.0, 620.0),
            pending: true,
            ..Default::default()
        };
        assert_eq!(c.layout(&native(560.0)).side, Side::Closed);
    }
    #[test]
    fn narrowing_is_temporary_and_never_replaces_the_main_preference() {
        let mut c = Controller {
            side: Side::Right,
            main: (560.0, 620.0),
            ..Default::default()
        };
        c.project(&native(1300.0));
        assert_eq!(c.main, (560.0, 620.0));
        c.project(&native(700.0));
        assert_eq!(c.side, Side::Inside);
        assert_eq!(c.main, (560.0, 620.0));
        assert_eq!(c.layout(&native(700.0)).main_width, 374.0);
        assert_eq!(c.close_target(&native(700.0)).inner.0, 560.0);
        c.project(&native(1000.0));
        assert_eq!(c.layout(&native(1000.0)).main_width, 560.0);
    }
    #[test]
    fn preview_memory_tracks_user_width_but_ignores_temporary_geometry() {
        let previous = native(1266.0);
        let mut c = Controller {
            side: Side::Right,
            main: (560.0, 620.0),
            preferred_preview: Some(700.0),
            normal: Some(Target::from_native(&previous)),
            preview_baseline: Some(Target::from_native(&previous)),
            ..Default::default()
        };
        c.pending = true;
        assert_eq!(c.remember_preview(&native(1466.0), false), None);
        c.pending = false;
        assert_eq!(c.remember_preview(&native(1000.0), true), None);
        let mut scaled = native(1466.0);
        scaled.scale = 2.0;
        assert_eq!(c.remember_preview(&scaled, false), None);
        assert_eq!(c.preferred_preview, Some(700.0));
        c.normal = Some(Target::from_native(&native(1466.0)));
        assert_eq!(c.remember_preview(&native(1466.0), false), Some(900.0));
        c.project(&native(700.0));
        assert_eq!(c.remember_preview(&native(700.0), false), None);
        assert_eq!(c.preferred_preview, Some(900.0));
        assert_eq!(c.close_target(&native(700.0)).inner.0, 560.0);
    }
    #[test]
    fn closing_restores_original_position_unless_the_user_moved_the_window() {
        let original = native(560.0);
        let mut expanded = native(1126.0);
        expanded.outer.x = 0.0;
        let mut c = Controller {
            side: Side::Right,
            main: original.inner,
            origin: Some(Target::from_native(&original)),
            normal: Some(Target::from_native(&expanded)),
            ..Default::default()
        };
        c.notice_move(&expanded);
        assert!(!c.user_moved);
        assert_eq!(c.close_target(&expanded).outer.x, 100.0);
        expanded.outer.x = 200.0;
        c.notice_move(&expanded);
        assert!(c.user_moved);
        assert_eq!(c.close_target(&expanded).outer.x, 200.0);
        expanded.outer.x = 0.0;
        c.notice_move(&expanded);
        assert!(c.user_moved);
    }
}
