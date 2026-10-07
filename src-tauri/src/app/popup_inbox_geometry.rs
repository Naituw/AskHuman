//! Left queue, normal answer area and right attachment preview share one geometry transaction.
use super::popup_preview::{Controller, Layout, SizeProjection};
use super::popup_preview_geometry::{Rect, Side};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager, PhysicalSize, WebviewWindow, Window};
const GAP: f64 = 6.0;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Canvas {
    pub left: f64,
    pub width: f64,
    pub height: f64,
    pub frozen: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Allocation {
    pub revision: u64,
    pub canvas: Option<Canvas>,
    pub sidebar_width: f64,
    pub main_width: f64,
    pub preview_width: f64,
    pub main_height: f64,
    pub frame: Frame,
    pub limited: bool,
}
impl Allocation {
    pub fn left_span(&self) -> f64 {
        self.sidebar_width + if self.sidebar_width > 0.0 { GAP } else { 0.0 }
    }
    pub fn preview_layout(&self) -> Layout {
        Layout {
            revision: self.revision,
            side: if self.preview_width > 0.0 {
                if self.limited {
                    Side::Inside
                } else {
                    Side::Right
                }
            } else {
                Side::Closed
            },
            main_width: self.main_width,
            main_height: self.main_height,
        }
    }
    pub fn minimum_width(&self) -> f64 {
        (self.left_span()
            + if self.preview_width > 0.0 {
                self.main_width + GAP + 320.0
            } else {
                super::popup_size::MIN_WIDTH
            })
        .min(self.frame.width)
    }
    fn responsive(&mut self) {
        let left_span = self.left_span();
        if let Some(canvas) = &mut self.canvas {
            canvas.width = canvas.left + self.frame.width - left_span;
            canvas.height = self.frame.height;
            canvas.frozen = false;
        }
    }
}
#[derive(Default)]
pub struct Geometry {
    preferred_main: Option<(f64, f64)>,
    preferred_sidebar: Option<f64>,
    preferred_preview: Option<f64>,
    anchor: Option<(f64, f64)>,
    sidebar: bool,
    preview: bool,
    revision: u64,
    pub pending: bool,
    canvas_right: f64,
    transition_left: f64,
    transition_frame: Option<Frame>,
    prepared: Option<Allocation>,
    published: Option<Allocation>,
    normal: Option<Frame>,
    special: bool,
    scale: Option<f64>,
    work: Option<Rect>,
    reconcile_queued: bool,
}
pub fn allocate(main: f64, left: f64, right: f64, available: f64) -> (f64, f64, f64) {
    let gaps = if left > 0.0 { GAP } else { 0.0 } + if right > 0.0 { GAP } else { 0.0 };
    let budget = (available - gaps).max(1.0);
    let (mut main, mut left, mut right) = (main, left, right);
    let mut deficit = (main + left + right - budget).max(0.0);
    let right_min = if right > 0.0 { 320.0 } else { 0.0 };
    let left_min = if left > 0.0 { 180.0 } else { 0.0 };
    for (value, minimum) in [
        (&mut right, right_min),
        (&mut left, left_min),
        (&mut main, 240.0),
    ] {
        let cut = deficit.min((*value - minimum).max(0.0));
        *value -= cut;
        deficit -= cut;
    }
    if deficit > 0.0 {
        let ratio = budget / (main + left + right);
        main *= ratio;
        left *= ratio;
        right *= ratio;
    }
    (main, left, right)
}
fn native(window: &Window) -> Result<(Frame, f64, Option<Rect>, bool), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    // In macOS single-WebView windows Tauri reads inner_size from WKWebView. Our fixed
    // viewport is deliberately larger than the visible window. Full-size content windows
    // include the titlebar, so the actual NSWindow outer extent is the visible content size.
    #[cfg(target_os = "macos")]
    let size = window.outer_size();
    #[cfg(not(target_os = "macos"))]
    let size = window.inner_size();
    let size = size.map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let pos = window
        .outer_position()
        .ok()
        .map(|p| p.to_logical::<f64>(scale));
    let frame = Frame {
        x: pos.map_or(0.0, |p| p.x),
        y: pos.map_or(0.0, |p| p.y),
        width: size.width,
        height: size.height,
    };
    let work = window
        .available_monitors()
        .ok()
        .into_iter()
        .flatten()
        .map(|m| {
            let work = m.work_area();
            #[cfg(target_os = "macos")]
            let divisor = m.scale_factor();
            #[cfg(not(target_os = "macos"))]
            let divisor = scale;
            Rect {
                x: work.position.x as f64 / divisor,
                y: work.position.y as f64 / divisor,
                width: work.size.width as f64 / divisor,
                height: work.size.height as f64 / divisor,
            }
        })
        .max_by(|a, b| {
            Rect {
                x: frame.x,
                y: frame.y,
                width: frame.width,
                height: frame.height,
            }
            .intersection(*a)
            .total_cmp(
                &Rect {
                    x: frame.x,
                    y: frame.y,
                    width: frame.width,
                    height: frame.height,
                }
                .intersection(*b),
            )
        });
    let special = window.is_maximized().unwrap_or(true) || window.is_fullscreen().unwrap_or(true);
    Ok((frame, scale, work, special))
}
fn state(window: &Window) -> tauri::State<'_, Mutex<Controller>> {
    window.app_handle().state::<Mutex<Controller>>()
}
fn publish(window: &Window, allocation: &Allocation) {
    let _ = window.emit("popup-inbox-layout", allocation);
    let _ = window.emit("popup-preview-layout", allocation.preview_layout());
}
fn webview(window: &Window) -> Result<WebviewWindow, String> {
    window
        .app_handle()
        .get_webview_window("popup")
        .ok_or("Popup window is unavailable".into())
}
pub async fn wait_idle(window: &Window) -> Result<(), String> {
    for _ in 0..150 {
        if !state(window).lock().unwrap().inbox.pending {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Err("preview geometry changed".into())
}
pub async fn prepare(
    window: &Window,
    sidebar: Option<bool>,
    preview: Option<bool>,
    sidebar_width: Option<f64>,
    main_extent: Option<f64>,
) -> Result<Allocation, String> {
    wait_idle(window).await?;
    wait_for_pointer(window).await;
    super::popup_pulse::cancel();
    let (frame, scale, work, special) = native(window)?;
    let config = crate::config::AppConfig::load_without_secrets()
        .channels
        .popup;
    let mut durable = None;
    let allocation = {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        let c = &mut owner.inbox;
        if c.pending {
            return Err("preview geometry changed".into());
        }
        let main = *c
            .preferred_main
            .get_or_insert_with(|| super::popup_size::restored_size(&config));
        c.preferred_preview
            .get_or_insert_with(|| super::popup_size::restored_preview_width(&config));
        c.preferred_sidebar
            .get_or_insert_with(|| super::popup_size::restored_sidebar_width(&config));
        let anchor = *c.anchor.get_or_insert((frame.x, frame.y));
        if let Some(sidebar) = sidebar {
            c.sidebar = sidebar;
        }
        if let Some(preview) = preview {
            c.preview = preview;
        }
        if let Some(width) = sidebar_width.filter(|n| n.is_finite()) {
            c.preferred_sidebar = Some(width.clamp(180.0, 600.0));
        }
        if let Some(extent) = main_extent.filter(|n| n.is_finite()) {
            let remaining = frame.width
                - if c.sidebar {
                    c.published.as_ref().map_or(240.0, |p| p.sidebar_width) + GAP
                } else {
                    0.0
                }
                - GAP;
            if remaining >= 740.0 && !special && c.published.as_ref().is_some_and(|p| !p.limited) {
                let width = extent.clamp(420.0, remaining - 320.0);
                c.preferred_main = Some((width, main.1));
                c.preferred_preview = Some(remaining - width);
                durable = Some(((width, main.1), remaining - width));
            }
        }
        let main = c.preferred_main.unwrap();
        let left = if c.sidebar {
            c.preferred_sidebar.unwrap()
        } else {
            0.0
        };
        let right = if c.preview {
            c.preferred_preview.unwrap()
        } else {
            0.0
        };
        let available = if special {
            frame.width
        } else {
            work.map_or(main.0 + left + right + 2.0 * GAP, |w| w.width)
        };
        let (mut main_width, sidebar_width, preview_width) =
            allocate(main.0, left, right, available);
        if special {
            main_width += (frame.width
                - main_width
                - sidebar_width
                - preview_width
                - if c.sidebar { GAP } else { 0.0 }
                - if c.preview { GAP } else { 0.0 })
            .max(0.0);
        }
        let width = main_width
            + sidebar_width
            + preview_width
            + if c.sidebar { GAP } else { 0.0 }
            + if c.preview { GAP } else { 0.0 };
        let mut target = Frame {
            x: anchor.0 - sidebar_width - if c.sidebar { GAP } else { 0.0 },
            y: anchor.1,
            width,
            height: main.1,
        };
        if let Some(work) = work {
            target.x = target.x.clamp(work.x, (work.right() - width).max(work.x));
            target.height = target.height.min(work.height);
            target.y = target
                .y
                .clamp(work.y, (work.bottom() - target.height).max(work.y));
        }
        if special {
            target = frame;
        }
        c.revision += 1;
        c.pending = true;
        c.special = special;
        c.scale = Some(scale);
        c.work = work;
        c.canvas_right = c
            .canvas_right
            .max(work.map_or(0.0, |w| w.width))
            .max(c.preferred_preview.unwrap() + GAP);
        let canvas_left = c
            .published
            .as_ref()
            .and_then(|previous| previous.canvas.as_ref())
            .map_or(606.0, |canvas| canvas.left);
        let canvas = cfg!(target_os = "macos").then(|| Canvas {
            left: canvas_left,
            width: canvas_left + main_width + c.canvas_right,
            height: target.height,
            frozen: true,
        });
        let allocation = Allocation {
            revision: c.revision,
            canvas,
            sidebar_width,
            main_width,
            preview_width,
            main_height: target.height,
            frame: target,
            limited: main_width < main.0 - 0.5
                || sidebar_width < left - 0.5
                || preview_width < right - 0.5,
        };
        c.transition_frame = Some(frame);
        c.transition_left = c.published.as_ref().map_or(0.0, Allocation::left_span);
        c.prepared = Some(allocation.clone());
        allocation
    };
    if let Some((main, preview)) = durable {
        super::popup_preview::save_dimensions(main, Some(preview));
    }
    let old_left = state(window).lock().unwrap().inbox.transition_left;
    if let Err(error) = super::popup_canvas::prepare(&webview(window)?, &allocation, old_left).await
    {
        let owner = state(window);
        let mut owner = owner.lock().unwrap();
        owner.inbox.pending = false;
        owner.inbox.prepared = None;
        return Err(error);
    }
    // A failed frontend cannot leave a permanently busy geometry owner.
    let owner = window.clone();
    let revision = allocation.revision;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if super::popup_preview::primary_button_down(&owner) {
            wait_for_pointer(&owner).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let expired = {
            let state = state(&owner);
            let mut state = state.lock().unwrap();
            let c = &mut state.inbox;
            if c.pending && c.revision == revision {
                c.pending = false;
                c.prepared = None;
                true
            } else {
                false
            }
        };
        if expired {
            let _ = owner.emit("popup-inbox-reconcile", ());
        }
    });
    publish(window, &allocation);
    Ok(allocation)
}
pub async fn commit_frame(window: &Window, revision: u64) -> Result<Allocation, String> {
    let (mut allocation, special, old_left, source) = {
        let state = state(window);
        let owner = state.lock().unwrap();
        let c = &owner.inbox;
        if !c.pending || c.revision != revision {
            return Err("preview geometry changed".into());
        }
        (
            c.prepared.clone().ok_or("preview geometry changed")?,
            c.special,
            c.transition_left,
            c.transition_frame.ok_or("preview geometry changed")?,
        )
    };
    let webview = webview(window)?;
    if !special {
        wait_for_pointer(window).await;
        {
            let state = state(window);
            let owner = state.lock().unwrap();
            if !owner.inbox.pending || owner.inbox.revision != revision {
                return Err("preview geometry changed".into());
            }
        }
        allocation.frame =
            super::popup_canvas::apply(&webview, &allocation, old_left, source).await?;
        let owner = state(window);
        let mut owner = owner.lock().unwrap();
        let c = &mut owner.inbox;
        if !c.pending || c.revision != revision {
            return Err("preview geometry changed".into());
        }
        c.anchor = Some((
            allocation.frame.x + allocation.left_span(),
            allocation.frame.y,
        ));
        c.prepared = Some(allocation.clone());
    }
    Ok(allocation)
}
pub async fn finish(window: &Window, revision: u64, arrival: bool) -> Result<Allocation, String> {
    let (mut allocation, special) = {
        let state = state(window);
        let owner = state.lock().unwrap();
        let c = &owner.inbox;
        if !c.pending || c.revision != revision {
            return Err("preview geometry changed".into());
        }
        (
            c.prepared.clone().ok_or("preview geometry changed")?,
            c.special,
        )
    };
    let webview = webview(window)?;
    // The frontend is still pinned to the final used widths while native removes the
    // offscreen reserve. Only then publish responsive CSS and allow ordinary edge drags.
    super::popup_canvas::resume(&webview, &allocation).await?;
    allocation.responsive();
    {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        let c = &mut owner.inbox;
        if c.revision == revision {
            c.pending = false;
            c.prepared = None;
            c.published = Some(allocation.clone());
            if !special {
                c.normal = Some(allocation.frame);
            }
        }
    }
    publish(window, &allocation);
    if arrival
        && window
            .app_handle()
            .state::<super::popup_inbox::Inbox>()
            .presented()
    {
        let _ = super::popup_transition::front(&webview).await;
        window
            .app_handle()
            .state::<super::popup_inbox::Inbox>()
            .notify_arrival(&webview);
    }
    Ok(allocation)
}
pub async fn commit(window: &Window, revision: u64, arrival: bool) -> Result<Allocation, String> {
    commit_frame(window, revision).await?;
    finish(window, revision, arrival).await
}
fn sidebar_resize(previous: &Allocation, frame: Frame, width: f64) -> Result<Allocation, String> {
    if !width.is_finite() || previous.sidebar_width <= 0.0 {
        return Err("Sidebar resize is unavailable".into());
    }
    let budget = frame.width
        - GAP
        - if previous.preview_width > 0.0 {
            previous.preview_width + GAP
        } else {
            0.0
        };
    if budget < 2.0 {
        return Err("Sidebar resize is unavailable".into());
    }
    let main_min = previous.main_width.clamp(1.0, super::popup_size::MIN_WIDTH);
    let sidebar_min = 180.0_f64.min((budget - main_min).max(1.0));
    let main_min = main_min.min(budget - sidebar_min);
    let sidebar_max = 600.0_f64.min(budget - main_min).max(sidebar_min);
    let sidebar_width = width.clamp(sidebar_min, sidebar_max);
    let mut allocation = previous.clone();
    allocation.sidebar_width = sidebar_width;
    allocation.main_width = budget - sidebar_width;
    allocation.main_height = frame.height;
    allocation.frame = frame;
    if let Some(canvas) = &mut allocation.canvas {
        // Move the DOM origin with the divider while leaving the native canvas and preview
        // at exactly the same screen position and viewport extent throughout the drag.
        canvas.left += sidebar_width - previous.sidebar_width;
    }
    allocation.responsive();
    Ok(allocation)
}
pub async fn resize_sidebar(
    window: &Window,
    width: f64,
    finished: bool,
) -> Result<Allocation, String> {
    wait_idle(window).await?;
    super::popup_pulse::cancel();
    let (frame, scale, work, special) = native(window)?;
    let view = webview(window)?;
    let (allocation, remember_main) = {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        let c = &mut owner.inbox;
        if c.pending || !c.sidebar {
            return Err("Sidebar resize is unavailable".into());
        }
        let previous = c
            .published
            .as_ref()
            .ok_or("Sidebar resize is unavailable")?;
        let remember_main = !special && !previous.limited;
        let mut allocation = sidebar_resize(previous, frame, width)?;
        c.revision += 1;
        allocation.revision = c.revision;
        c.pending = true;
        c.prepared = Some(allocation.clone());
        (allocation, remember_main)
    };
    // The native view frames are unchanged; only the minimum size follows the new split.
    // Never prepare a reserved viewport or set the outer window frame for divider motion.
    if let Err(error) = super::popup_canvas::resume(&view, &allocation).await {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        owner.inbox.pending = false;
        owner.inbox.prepared = None;
        return Err(error);
    }
    {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        let c = &mut owner.inbox;
        c.pending = false;
        c.prepared = None;
        c.published = Some(allocation.clone());
        c.preferred_sidebar = Some(allocation.sidebar_width);
        if remember_main {
            c.preferred_main = Some((allocation.main_width, allocation.main_height));
            c.normal = Some(frame);
        }
        c.anchor = Some((frame.x + allocation.left_span(), frame.y));
        c.scale = Some(scale);
        c.work = work;
        c.special = special;
    }
    publish(window, &allocation);
    if finished {
        // Persist only the final pointer position, avoiding config writes on every frame.
        let mut config = crate::config::AppConfig::load_without_secrets();
        if config.channels.popup.remember_size {
            config.channels.popup.sidebar_width = allocation.sidebar_width;
            if remember_main {
                config.channels.popup.width = allocation.main_width;
                config.channels.popup.height = allocation.main_height;
            }
            config
                .save()
                .map_err(|e| format!("Unable to remember sidebar width: {e}"))?;
        }
    }
    Ok(allocation)
}
pub fn published(window: &Window) -> Option<Layout> {
    let owner = state(window);
    let owner = owner.lock().ok()?;
    let c = &owner.inbox;
    c.prepared
        .as_ref()
        .or(c.published.as_ref())
        .map(Allocation::preview_layout)
}
pub fn preview_bounds(window: &Window) -> Option<(f64, f64, f64)> {
    let owner = state(window);
    let owner = owner.lock().ok()?;
    let c = &owner.inbox;
    let allocation = c.prepared.as_ref().or(c.published.as_ref())?;
    let left = allocation
        .canvas
        .as_ref()
        .map_or(allocation.left_span(), |c| c.left);
    Some((
        left + allocation.main_width,
        left + allocation.main_width
            + if allocation.preview_width > 0.0 {
                GAP + allocation.preview_width
            } else {
                0.0
            },
        allocation.main_height,
    ))
}
pub fn resized(window: &Window, event: PhysicalSize<u32>) -> Option<SizeProjection> {
    let (frame, scale, work, special) = native(window).ok()?;
    // AppKit sends intermediate resize events while zooming/restoring, before isZoomed
    // reflects the destination. Only a real edge drag may update durable dimensions.
    #[cfg(target_os = "macos")]
    let user_resize = window.ns_window().ok().is_some_and(|pointer| unsafe {
        use objc2::{msg_send, runtime::AnyObject};
        let native = pointer as *mut AnyObject;
        let live: bool = msg_send![native, inLiveResize];
        live
    });
    #[cfg(not(target_os = "macos"))]
    let user_resize = true;
    #[cfg(not(target_os = "macos"))]
    if window.inner_size().ok()? != event {
        return None;
    }
    #[cfg(target_os = "macos")]
    let _ = event;
    let state = state(window);
    let mut owner = state.lock().ok()?;
    let c = &mut owner.inbox;
    if c.pending {
        return None;
    }
    let previous = c.published.clone()?;
    let restoring = c.special && !special;
    let dpi_changed = c.scale.is_some_and(|old| (old - scale).abs() > 0.001);
    let transient = special || restoring || dpi_changed || !user_resize;
    if (frame.width - previous.frame.width).abs() <= 1.0 / scale
        && (frame.height - previous.frame.height).abs() <= 1.0 / scale
    {
        return None;
    }
    let main = c.preferred_main?;
    let constrained = work.is_some_and(|work| {
        frame.width >= work.width - 2.0
            && main.0
                + if c.sidebar {
                    c.preferred_sidebar.unwrap_or(240.0) + GAP
                } else {
                    0.0
                }
                + if c.preview {
                    c.preferred_preview.unwrap_or(700.0) + GAP
                } else {
                    0.0
                }
                > work.width + 2.0
    });
    let (main_width, left, right) = resize_regions(&previous, frame.width);
    let mut preview_memory = None;
    if !transient {
        // Sidebar preferences are changed only by its divider. A closed preview gives the
        // entire width delta to the answer; an open preview receives that delta itself.
        if !c.preview && !constrained {
            c.preferred_main = Some((main_width, frame.height));
        } else {
            c.preferred_main = Some((main.0, frame.height));
            if c.preview && right >= 320.0 && !constrained {
                c.preferred_preview = Some(right);
                preview_memory = Some(right);
            }
        }
    }
    c.special = special;
    c.scale = Some(scale);
    c.work = work;
    c.revision += 1;
    let canvas_left = previous.canvas.as_ref().map_or(606.0, |canvas| canvas.left);
    let canvas = cfg!(target_os = "macos").then(|| Canvas {
        left: canvas_left,
        width: canvas_left + frame.width - previous.left_span(),
        height: frame.height,
        frozen: false,
    });
    let layout = Allocation {
        revision: c.revision,
        canvas,
        sidebar_width: left,
        main_width,
        preview_width: right,
        main_height: frame.height,
        frame,
        limited: main_width < c.preferred_main?.0 - 0.5,
    };
    c.anchor = Some((frame.x + left + if c.sidebar { GAP } else { 0.0 }, frame.y));
    if !transient {
        c.normal = Some(frame);
    }
    c.published = Some(layout.clone());
    let normal = c.preferred_main?;
    drop(owner);
    // AppKit resizes the canvas, WebView and native preview synchronously. This event only
    // updates authoritative geometry and persistence; it must never reset their frames.
    publish(window, &layout);
    if dpi_changed || (restoring && !cfg!(target_os = "macos")) {
        let _ = window.emit("popup-inbox-reconcile", ());
    }
    if transient {
        return None;
    }
    Some(SizeProjection {
        main: (
            (normal.0 * scale).round() as u32,
            (normal.1 * scale).round() as u32,
        ),
        preview: preview_memory,
    })
}
fn resize_regions(previous: &Allocation, width: f64) -> (f64, f64, f64) {
    let available = (width - previous.left_span()).max(1.0);
    if previous.preview_width > 0.0 {
        let main = previous.main_width.min((available - GAP - 1.0).max(1.0));
        (
            main,
            previous.sidebar_width,
            (available - main - GAP).max(1.0),
        )
    } else {
        (available, previous.sidebar_width, 0.0)
    }
}
pub fn moved(window: &Window) {
    let Ok((frame, _, work, special)) = native(window) else {
        return;
    };
    let state = state(window);
    let mut owner = state.lock().unwrap();
    let c = &mut owner.inbox;
    if c.pending || special {
        return;
    }
    if let Some(layout) = c.published.as_ref() {
        c.anchor = Some((
            frame.x + layout.sidebar_width + if c.sidebar { GAP } else { 0.0 },
            frame.y,
        ));
    }
    let changed = match (c.work, work) {
        (Some(old), Some(new)) => old != new,
        (None, None) => false,
        _ => true,
    };
    c.work = work;
    let reconcile = changed && !c.reconcile_queued;
    if reconcile {
        c.reconcile_queued = true;
    }
    drop(owner);
    if reconcile {
        let window = window.clone();
        tauri::async_runtime::spawn(async move {
            wait_for_pointer(&window).await;
            self::state(&window).lock().unwrap().inbox.reconcile_queued = false;
            let _ = window.emit("popup-inbox-reconcile", ());
        });
    }
}
async fn wait_for_pointer(window: &Window) {
    while super::popup_preview::primary_button_down(window) {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
pub async fn reset(window: &Window) -> Result<(), String> {
    super::popup_pulse::cancel();
    let allocation = prepare(window, Some(false), Some(false), None, None).await?;
    commit(window, allocation.revision, false).await?;
    let owner = state(window);
    let mut owner = owner.lock().unwrap();
    let canvas_right = owner.inbox.canvas_right;
    owner.inbox = Geometry {
        canvas_right,
        ..Geometry::default()
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(preview: f64) -> Allocation {
        Allocation {
            revision: 1,
            canvas: Some(Canvas {
                left: 606.0,
                width: 4000.0,
                height: 620.0,
                frozen: true,
            }),
            sidebar_width: 240.0,
            main_width: 560.0,
            preview_width: preview,
            main_height: 620.0,
            frame: Frame {
                x: 0.0,
                y: 0.0,
                width: 806.0 + if preview > 0.0 { preview + GAP } else { 0.0 },
                height: 620.0,
            },
            limited: false,
        }
    }
    #[test]
    fn outer_resize_keeps_sidebar_and_gives_delta_to_main_without_preview() {
        let previous = layout(0.0);
        assert_eq!(resize_regions(&previous, 966.0), (720.0, 240.0, 0.0));
        assert_eq!(resize_regions(&previous, 666.0), (420.0, 240.0, 0.0));
        assert_eq!(previous.minimum_width(), 666.0);
    }
    #[test]
    fn outer_resize_keeps_main_and_sidebar_when_preview_is_open() {
        let previous = layout(700.0);
        assert_eq!(resize_regions(&previous, 1672.0), (560.0, 240.0, 860.0));
        assert_eq!(resize_regions(&previous, 1132.0), (560.0, 240.0, 320.0));
        assert_eq!(previous.minimum_width(), 1132.0);
    }
    #[test]
    fn responsive_canvas_has_no_unused_right_reserve_and_keeps_main_origin() {
        let mut allocation = layout(700.0);
        allocation.responsive();
        let canvas = allocation.canvas.unwrap();
        assert!(!canvas.frozen);
        assert_eq!(canvas.left, 606.0);
        assert_eq!(canvas.width, 1872.0);
    }
    #[test]
    fn sidebar_drag_redistributes_inside_fixed_window_and_native_viewport() {
        for preview in [0.0, 700.0] {
            let mut previous = layout(preview);
            previous.responsive();
            let initial = previous.clone();
            for width in [340.0, 180.0, 380.0, 260.0, 240.0] {
                let next = sidebar_resize(&previous, previous.frame, width).unwrap();
                assert_eq!(next.sidebar_width, width);
                assert_eq!(next.sidebar_width + next.main_width, 800.0);
                assert_eq!(next.preview_width, preview);
                assert_eq!(
                    serde_json::to_value(next.frame).unwrap(),
                    serde_json::to_value(initial.frame).unwrap()
                );
                let canvas = next.canvas.as_ref().unwrap();
                let original = initial.canvas.as_ref().unwrap();
                assert_eq!(canvas.width, original.width);
                assert_eq!(
                    next.left_span() - canvas.left,
                    initial.left_span() - original.left
                );
                assert_eq!(
                    canvas.left + next.main_width,
                    original.left + initial.main_width
                );
                assert!(!canvas.frozen);
                previous = next;
            }
        }
    }
    #[test]
    fn sidebar_drag_respects_main_floor_and_existing_constrained_layout() {
        let previous = layout(0.0);
        let wide = sidebar_resize(&previous, previous.frame, 900.0).unwrap();
        assert_eq!((wide.sidebar_width, wide.main_width), (380.0, 420.0));
        let narrow = sidebar_resize(&previous, previous.frame, -30.0).unwrap();
        assert_eq!((narrow.sidebar_width, narrow.main_width), (180.0, 620.0));
        let mut constrained = previous;
        constrained.sidebar_width = 150.0;
        constrained.main_width = 300.0;
        constrained.frame.width = 456.0;
        constrained.limited = true;
        let result = sidebar_resize(&constrained, constrained.frame, 500.0).unwrap();
        assert_eq!((result.sidebar_width, result.main_width), (150.0, 300.0));
        assert!(result.limited);
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(sidebar_resize(&constrained, constrained.frame, invalid).is_err());
        }
        constrained.sidebar_width = 0.0;
        assert!(sidebar_resize(&constrained, constrained.frame, 240.0).is_err());
    }
    #[test]
    fn reduces_preview_then_sidebar_then_main() {
        assert_eq!(allocate(560.0, 240.0, 700.0, 1200.0), (560.0, 240.0, 388.0));
        assert_eq!(allocate(560.0, 240.0, 700.0, 1100.0), (560.0, 208.0, 320.0));
        assert_eq!(allocate(560.0, 240.0, 700.0, 900.0), (388.0, 180.0, 320.0));
        assert_eq!(allocate(560.0, 0.0, 0.0, 900.0), (560.0, 0.0, 0.0));
    }
}
