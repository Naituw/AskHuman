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
    /// The width a Sidebar has, or would get if one appeared: the user's last choice or the default.
    /// A single request has no Sidebar, yet its small window is the list, so the page needs the width.
    pub sidebar_preferred: f64,
    pub main_width: f64,
    pub preview_width: f64,
    pub main_height: f64,
    pub frame: Frame,
    pub limited: bool,
    /// The window is the small mini-list window (docs/specs/popup-inbox-collapse.md). The widths and
    /// `main_height` above still describe the pinned expanded layout the page keeps laid out; only
    /// `frame` is small. The viewport stays frozen for as long as this is set.
    pub collapsed: bool,
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
/// The size of the small window, measured by the page in its final small layout. Never persisted.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mini {
    pub width: f64,
    pub height: f64,
}

/// Which window form a layout transaction asks for.
#[derive(Clone, Copy, Debug, Default)]
pub enum Form {
    /// Keep whatever form the window has (callers that only change the attachment preview).
    #[default]
    Keep,
    Expanded,
    Collapsed(Mini),
}

/// What kind of native change a prepared transaction needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// Sidebar, preview or outer edge: an instant frame change that keeps the answer body in place.
    Plain,
    /// Expanded window shrinks to the small one.
    Collapse,
    /// The small window changes size (its list got shorter or longer).
    MiniResize,
    /// The small window grows back to the expanded layout.
    Expand,
}

/// How a form change is animated. Without one the change is instant.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Animation {
    pub duration_ms: f64,
    /// CSS-style `cubic-bezier(x1, y1, x2, y2)`; the driver default when absent.
    pub curve: Option<[f64; 4]>,
}

/// A layout transaction's inputs; `None` leaves that part as it is.
#[derive(Clone, Copy, Debug, Default)]
pub struct Request {
    pub sidebar: Option<bool>,
    pub preview: Option<bool>,
    pub sidebar_width: Option<f64>,
    pub main_extent: Option<f64>,
    pub form: Form,
    /// Files are being dragged over the window: the primary button is held on purpose, so the
    /// transaction must not wait for its release (spring-loaded expansion).
    pub dragging: bool,
    /// The round is over: put the window where the next round starts instead of anchoring it.
    pub home: bool,
}

/// The window as the OS reports it when a transaction is planned.
#[derive(Clone, Copy, Debug)]
struct Native {
    frame: Frame,
    scale: f64,
    work: Option<Rect>,
    special: bool,
}

/// Saved or default dimensions, used the first time they are needed.
struct Defaults {
    main: (f64, f64),
    preview: f64,
    sidebar: f64,
}

struct Planned {
    allocation: Allocation,
    /// Main and preview dimensions the user chose, to be saved before the command returns.
    durable: Option<((f64, f64), f64)>,
}

#[derive(Debug, PartialEq, Eq)]
enum PlanError {
    Busy,
    Invalid(&'static str),
}

/// What the window's left edge or the answer body stays fixed to during a transaction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Anchor {
    /// The answer body keeps its screen position; the window grows to the left (D5).
    Body,
    /// The window's own top-left corner keeps its position (collapse and expand).
    Window,
    /// The window goes where the next round starts (`anchor - drift`).
    Home,
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
    /// Kind of the prepared transaction, which decides how `commit_frame` moves the window.
    transition: Option<Transition>,
    prepared: Option<Allocation>,
    published: Option<Allocation>,
    normal: Option<Frame>,
    special: bool,
    scale: Option<f64>,
    work: Option<Rect>,
    reconcile_queued: bool,
    /// How far the body anchor has moved only because window-anchored changes (collapse / expand)
    /// changed the width of the column left of the body: `anchor.x - home.x`, where `home` is where
    /// an idle window belongs. A user move shifts anchor and home alike, so it never counts. `reset`
    /// returns to `anchor.x - drift`, which is why a new round starts where the last one did.
    drift: f64,
    /// The part of `drift` the prepared transaction adds when it commits.
    drift_step: f64,
    /// The prepared transaction does not wait for a held primary button.
    dragging: bool,
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
impl Geometry {
    fn is_collapsed(&self) -> bool {
        self.published.as_ref().is_some_and(|p| p.collapsed)
    }

    /// Resize events are not user edge drags while a transaction runs or the window is the small
    /// one (which has no edges to drag), so they must not touch preferences or published state.
    fn ignores_resize(&self) -> bool {
        self.pending || self.is_collapsed()
    }

    /// Decide the next window layout and start a transaction (`pending`). All state it changes is
    /// the owner's, so the same code runs under the lock in production and in the geometry tests.
    fn plan(
        &mut self,
        native: Native,
        request: &Request,
        defaults: &Defaults,
    ) -> Result<Planned, PlanError> {
        if self.pending {
            return Err(PlanError::Busy);
        }
        let Native {
            frame,
            scale,
            work,
            special,
        } = native;
        let was_collapsed = self.is_collapsed();
        let mini = match request.form {
            Form::Expanded => None,
            Form::Collapsed(mini) => Some(mini),
            Form::Keep => self
                .published
                .as_ref()
                .filter(|p| p.collapsed)
                .map(|p| Mini {
                    width: p.frame.width,
                    height: p.frame.height,
                }),
        };
        if let Some(mini) = mini {
            if !(mini.width.is_finite()
                && mini.height.is_finite()
                && mini.width >= 1.0
                && mini.height >= 1.0)
            {
                return Err(PlanError::Invalid("Small window size is invalid"));
            }
            if special {
                return Err(PlanError::Invalid(
                    "The window cannot shrink while it is maximized or full screen",
                ));
            }
        }
        let mode = if request.home {
            Anchor::Home
        } else if mini.is_some() || was_collapsed {
            Anchor::Window
        } else {
            Anchor::Body
        };
        let main = *self.preferred_main.get_or_insert(defaults.main);
        self.preferred_preview.get_or_insert(defaults.preview);
        self.preferred_sidebar.get_or_insert(defaults.sidebar);
        let anchor = *self.anchor.get_or_insert((frame.x, frame.y));
        if let Some(sidebar) = request.sidebar {
            self.sidebar = sidebar;
        }
        if let Some(preview) = request.preview {
            self.preview = preview;
        }
        if let Some(width) = request.sidebar_width.filter(|n| n.is_finite()) {
            self.preferred_sidebar = Some(width.clamp(180.0, 600.0));
        }
        let mut durable = None;
        // The small window never changes what the expanded one remembers.
        if let Some(extent) = request
            .main_extent
            .filter(|n| n.is_finite() && mini.is_none())
        {
            let remaining = frame.width
                - if self.sidebar {
                    self.published.as_ref().map_or(240.0, |p| p.sidebar_width) + GAP
                } else {
                    0.0
                }
                - GAP;
            if remaining >= 740.0 && !special && self.published.as_ref().is_some_and(|p| !p.limited)
            {
                let width = extent.clamp(420.0, remaining - 320.0);
                self.preferred_main = Some((width, main.1));
                self.preferred_preview = Some(remaining - width);
                durable = Some(((width, main.1), remaining - width));
            }
        }
        let main = self.preferred_main.unwrap();
        let left = if self.sidebar {
            self.preferred_sidebar.unwrap()
        } else {
            0.0
        };
        let right = if self.preview {
            self.preferred_preview.unwrap()
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
                - if self.sidebar { GAP } else { 0.0 }
                - if self.preview { GAP } else { 0.0 })
            .max(0.0);
        }
        let width = main_width
            + sidebar_width
            + preview_width
            + if self.sidebar { GAP } else { 0.0 }
            + if self.preview { GAP } else { 0.0 };
        let left_span_new = sidebar_width + if self.sidebar { GAP } else { 0.0 };
        let (anchor_x, anchor_y) = match mode {
            Anchor::Body => anchor,
            Anchor::Window => (frame.x + left_span_new, frame.y),
            Anchor::Home => (anchor.0 - self.drift, anchor.1),
        };
        let mut target = Frame {
            x: anchor_x - left_span_new,
            y: anchor_y,
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
        // The page keeps the expanded layout (and its pinned viewport) while the window is small.
        let mut window_frame = target;
        if let Some(mini) = mini {
            window_frame = Frame {
                x: frame.x,
                y: frame.y,
                width: mini.width,
                height: mini.height,
            };
            if let Some(work) = work {
                window_frame.width = window_frame.width.min(work.width);
                window_frame.height = window_frame.height.min(work.height);
                window_frame.x = window_frame
                    .x
                    .clamp(work.x, (work.right() - window_frame.width).max(work.x));
                window_frame.y = window_frame
                    .y
                    .clamp(work.y, (work.bottom() - window_frame.height).max(work.y));
            }
        }
        let left_span_old = self.published.as_ref().map_or(0.0, Allocation::left_span);
        self.revision += 1;
        self.pending = true;
        self.special = special;
        self.scale = Some(scale);
        self.work = work;
        self.canvas_right = self
            .canvas_right
            .max(work.map_or(0.0, |w| w.width))
            .max(self.preferred_preview.unwrap() + GAP);
        let previous_left = self
            .published
            .as_ref()
            .and_then(|previous| previous.canvas.as_ref())
            .map_or(606.0, |canvas| canvas.left);
        // A window-anchored change keeps the viewport origin on screen (the window edge sweeps over
        // painted content) and moves the body origin by the change of the column left of it.
        let canvas_left = if mode == Anchor::Window {
            previous_left - left_span_old + left_span_new
        } else {
            previous_left
        };
        let canvas = cfg!(target_os = "macos").then(|| Canvas {
            left: canvas_left,
            width: canvas_left + main_width + self.canvas_right,
            height: target.height,
            frozen: true,
        });
        let allocation = Allocation {
            revision: self.revision,
            canvas,
            sidebar_width,
            sidebar_preferred: self.preferred_sidebar.unwrap_or(sidebar_width),
            main_width,
            preview_width,
            main_height: target.height,
            frame: window_frame,
            limited: main_width < main.0 - 0.5
                || sidebar_width < left - 0.5
                || preview_width < right - 0.5,
            collapsed: mini.is_some(),
        };
        self.transition = Some(match (mini.is_some(), was_collapsed) {
            (true, false) => Transition::Collapse,
            (true, true) => Transition::MiniResize,
            (false, true) => Transition::Expand,
            (false, false) => Transition::Plain,
        });
        self.transition_frame = Some(frame);
        // Where the body origin sits in the window when the viewport is pinned for the change.
        self.transition_left = if mode == Anchor::Window {
            left_span_new
        } else {
            left_span_old
        };
        self.drift_step = if mode == Anchor::Window {
            left_span_new - left_span_old
        } else {
            0.0
        };
        self.dragging = request.dragging;
        self.prepared = Some(allocation.clone());
        Ok(Planned {
            allocation,
            durable,
        })
    }

    /// The window now has `allocation.frame`: the body anchor follows it and a window-anchored
    /// change books how far it moved the body.
    fn committed(&mut self, allocation: &Allocation) {
        self.anchor = Some((
            allocation.frame.x + allocation.left_span(),
            allocation.frame.y,
        ));
        self.drift += self.drift_step;
        self.drift_step = 0.0;
        self.prepared = Some(allocation.clone());
    }

    /// Publish the prepared layout as the settled one. Stale revisions are ignored.
    fn finished(&mut self, revision: u64, allocation: &Allocation) {
        if self.revision != revision {
            return;
        }
        self.pending = false;
        self.prepared = None;
        self.transition = None;
        self.drift_step = 0.0;
        self.published = Some(allocation.clone());
        if !self.special && !allocation.collapsed {
            self.normal = Some(allocation.frame);
        }
    }

    /// The user (or the OS) moved the window. Returns whether the work area changed and the page
    /// has to be asked to reconcile. The anchor follows the window only through the settled layout,
    /// so it also holds for the small window, whose published layout keeps the expanded columns.
    fn moved_to(&mut self, frame: Frame, work: Option<Rect>, special: bool) -> bool {
        if self.pending || special {
            return false;
        }
        if let Some(layout) = self.published.as_ref() {
            self.anchor = Some((frame.x + layout.left_span(), frame.y));
        }
        let changed = match (self.work, work) {
            (Some(old), Some(new)) => old != new,
            (None, None) => false,
            _ => true,
        };
        self.work = work;
        let reconcile = changed && !self.reconcile_queued;
        if reconcile {
            self.reconcile_queued = true;
        }
        reconcile
    }
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
fn geometry_conflict(stage: &'static str) -> String {
    crate::daemon::lifecycle::log_runtime_event("popup_geometry", stage, None);
    "preview geometry changed".into()
}
pub async fn wait_idle(window: &Window) -> Result<(), String> {
    for _ in 0..150 {
        if !state(window).lock().unwrap().inbox.pending {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Err(geometry_conflict("idle_wait_expired"))
}
pub async fn prepare(window: &Window, request: Request) -> Result<Allocation, String> {
    wait_idle(window).await?;
    // A held primary button normally means an edge drag in progress; a file drag over the window
    // holds it on purpose, and the spring-loaded expansion has to happen while it is held.
    if !request.dragging {
        wait_for_pointer(window).await;
    }
    let (frame, scale, work, special) = native(window)?;
    let config = crate::config::AppConfig::load_without_secrets()
        .channels
        .popup;
    let defaults = Defaults {
        main: super::popup_size::restored_size(&config),
        preview: super::popup_size::restored_preview_width(&config),
        sidebar: super::popup_size::restored_sidebar_width(&config),
    };
    let native = Native {
        frame,
        scale,
        work,
        special,
    };
    let planned = {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        owner.inbox.plan(native, &request, &defaults)
    };
    let Planned {
        allocation,
        durable,
    } = match planned {
        Ok(planned) => planned,
        Err(PlanError::Busy) => return Err(geometry_conflict("prepare_busy")),
        Err(PlanError::Invalid(reason)) => return Err(reason.into()),
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
        owner.inbox.transition = None;
        owner.inbox.drift_step = 0.0;
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
                c.transition = None;
                c.drift_step = 0.0;
                true
            } else {
                false
            }
        };
        if expired {
            crate::daemon::lifecycle::log_runtime_event(
                "popup_geometry",
                "preparation_expired",
                None,
            );
            let _ = owner.emit("popup-inbox-reconcile", ());
        }
    });
    publish(window, &allocation);
    Ok(allocation)
}
pub async fn commit_frame(
    window: &Window,
    revision: u64,
    animation: Option<Animation>,
) -> Result<Allocation, String> {
    let (mut allocation, special, old_left, source, transition, dragging) = {
        let state = state(window);
        let owner = state.lock().unwrap();
        let c = &owner.inbox;
        if !c.pending || c.revision != revision {
            return Err(geometry_conflict("commit_stale"));
        }
        (
            c.prepared
                .clone()
                .ok_or_else(|| geometry_conflict("commit_missing_preparation"))?,
            c.special,
            c.transition_left,
            c.transition_frame
                .ok_or_else(|| geometry_conflict("commit_missing_frame"))?,
            c.transition.unwrap_or(Transition::Plain),
            c.dragging,
        )
    };
    let webview = webview(window)?;
    if !special {
        if !dragging {
            wait_for_pointer(window).await;
        }
        {
            let state = state(window);
            let owner = state.lock().unwrap();
            if !owner.inbox.pending || owner.inbox.revision != revision {
                return Err(geometry_conflict("commit_stale_after_pointer"));
            }
        }
        allocation.frame = match transition {
            Transition::Plain => {
                super::popup_canvas::apply(&webview, &allocation, old_left, source).await?
            }
            // Form changes run the display-link driver; without an animation it is a single frame.
            _ => {
                super::popup_canvas::animate(&webview, &allocation, transition, source, animation)
                    .await?
            }
        };
        let owner = state(window);
        let mut owner = owner.lock().unwrap();
        let c = &mut owner.inbox;
        if !c.pending || c.revision != revision {
            return Err(geometry_conflict("commit_stale_after_apply"));
        }
        c.committed(&allocation);
    }
    Ok(allocation)
}
pub async fn finish(window: &Window, revision: u64, arrival: bool) -> Result<Allocation, String> {
    let mut allocation = {
        let state = state(window);
        let owner = state.lock().unwrap();
        let c = &owner.inbox;
        if !c.pending || c.revision != revision {
            return Err(geometry_conflict("finish_stale"));
        }
        c.prepared
            .clone()
            .ok_or_else(|| geometry_conflict("finish_missing_preparation"))?
    };
    let webview = webview(window)?;
    // The small window keeps its viewport pinned: the page stays laid out for the expanded window
    // and the window shows only the top-left corner of it.
    if !allocation.collapsed {
        // The frontend is still pinned to the final used widths while native removes the
        // offscreen reserve. Only then publish responsive CSS and allow ordinary edge drags.
        super::popup_canvas::resume(&webview, &allocation).await?;
        allocation.responsive();
    }
    {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        owner.inbox.finished(revision, &allocation);
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
pub async fn commit(
    window: &Window,
    revision: u64,
    animation: Option<Animation>,
    arrival: bool,
) -> Result<Allocation, String> {
    commit_frame(window, revision, animation).await?;
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
    allocation.sidebar_preferred = sidebar_width;
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
    let (frame, scale, work, special) = native(window)?;
    let view = webview(window)?;
    let (allocation, remember_main) = {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        let c = &mut owner.inbox;
        if c.pending || !c.sidebar || c.is_collapsed() {
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
fn preview_resize(
    previous: &Allocation,
    frame: Frame,
    extent: f64,
    special: bool,
) -> Result<(Allocation, bool), String> {
    let available = frame.width - previous.left_span();
    let budget = available - GAP;
    if !extent.is_finite() || previous.preview_width <= 0.0 || budget < 2.0 {
        return Err("Preview resize is unavailable".into());
    }
    let durable = !special && !previous.limited && budget >= 740.0;
    let main_width = if durable {
        extent.clamp(420.0, budget - 320.0)
    } else {
        super::popup_preview_geometry::inside_main_width(available, extent, None)
    };
    let mut allocation = previous.clone();
    allocation.main_width = main_width;
    allocation.preview_width = budget - main_width;
    allocation.main_height = frame.height;
    allocation.frame = frame;
    allocation.responsive();
    Ok((allocation, durable))
}
pub async fn resize_preview(
    window: &Window,
    request_id: &str,
    extent: f64,
    finished: bool,
) -> Result<Allocation, String> {
    wait_idle(window).await?;
    super::popup_preview::request(window, request_id)?;
    let (frame, scale, work, special) = native(window)?;
    let view = webview(window)?;
    let (allocation, durable) = {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        let c = &mut owner.inbox;
        if c.pending || !c.preview || c.is_collapsed() {
            return Err("Preview resize is unavailable".into());
        }
        let previous = c
            .published
            .as_ref()
            .ok_or("Preview resize is unavailable")?;
        let (mut allocation, durable) = preview_resize(previous, frame, extent, special)?;
        c.revision += 1;
        allocation.revision = c.revision;
        c.pending = true;
        c.prepared = Some(allocation.clone());
        (allocation, durable)
    };
    // A divider changes the split and minimum width only. Keep the real window and
    // responsive native viewport intact, and never wait for the mouse to be released.
    let result = super::popup_canvas::resume(&view, &allocation).await;
    {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        let c = &mut owner.inbox;
        c.pending = false;
        c.prepared = None;
        result?;
        super::popup_preview::request(window, request_id)?;
        c.published = Some(allocation.clone());
        if durable {
            c.preferred_main = Some((allocation.main_width, allocation.main_height));
            c.preferred_preview = Some(allocation.preview_width);
            c.normal = Some(frame);
        }
        c.anchor = Some((frame.x + allocation.left_span(), frame.y));
        c.scale = Some(scale);
        c.work = work;
        c.special = special;
    }
    publish(window, &allocation);
    if finished && durable {
        super::popup_preview::save_dimensions(
            (allocation.main_width, allocation.main_height),
            Some(allocation.preview_width),
        );
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
    // The small window has no edges to drag and its size is never remembered (R10).
    if c.ignores_resize() {
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
        sidebar_preferred: c.preferred_sidebar.unwrap_or(left),
        main_width,
        preview_width: right,
        main_height: frame.height,
        frame,
        limited: main_width < c.preferred_main?.0 - 0.5,
        collapsed: false,
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
    let reconcile = {
        let state = state(window);
        let mut owner = state.lock().unwrap();
        owner.inbox.moved_to(frame, work, special)
    };
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
    // The window is hidden by now; it goes back to where the next round starts, in one instant
    // transaction that also restores the expanded form when the round ended on the small window.
    let allocation = prepare(
        window,
        Request {
            sidebar: Some(false),
            preview: Some(false),
            form: Form::Expanded,
            home: true,
            ..Request::default()
        },
    )
    .await?;
    commit(window, allocation.revision, None, false).await?;
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
            sidebar_preferred: 240.0,
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
            collapsed: false,
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
    fn preview_drag_keeps_window_sidebar_and_native_viewport_unchanged() {
        for sidebar in [0.0, 240.0] {
            let mut previous = layout(700.0);
            previous.sidebar_width = sidebar;
            previous.frame.width = previous.left_span() + 560.0 + GAP + 700.0;
            previous.responsive();
            let initial = previous.clone();
            for extent in [720.0, 840.0, 420.0, 500.0, 560.0] {
                let (next, durable) =
                    preview_resize(&previous, previous.frame, extent, false).unwrap();
                assert!(durable);
                assert_eq!(next.main_width, extent);
                assert_eq!(next.main_width + next.preview_width, 1260.0);
                assert_eq!(next.sidebar_width, initial.sidebar_width);
                assert_eq!(
                    serde_json::to_value(next.frame).unwrap(),
                    serde_json::to_value(initial.frame).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(&next.canvas).unwrap(),
                    serde_json::to_value(&initial.canvas).unwrap()
                );
                previous = next;
            }
        }
    }
    #[test]
    fn preview_drag_clamps_normal_widths_and_keeps_limited_or_special_adjustments_temporary() {
        let previous = layout(700.0);
        for (wanted, expected) in [(-30.0, 420.0), (3000.0, 940.0)] {
            let (next, durable) = preview_resize(&previous, previous.frame, wanted, false).unwrap();
            assert!(durable);
            assert_eq!(next.main_width, expected);
            assert!(next.preview_width >= 320.0);
        }
        let (special, durable) = preview_resize(&previous, previous.frame, -30.0, true).unwrap();
        assert!(!durable);
        assert_eq!(special.main_width, 240.0);
        let mut limited = previous.clone();
        limited.limited = true;
        limited.frame.width = limited.left_span() + 600.0;
        let (next, durable) = preview_resize(&limited, limited.frame, 900.0, false).unwrap();
        assert!(!durable);
        assert_eq!((next.main_width, next.preview_width), (297.0, 297.0));
        limited.frame.width = limited.left_span() + 400.0;
        let (next, durable) = preview_resize(&limited, limited.frame, 900.0, false).unwrap();
        assert!(!durable);
        assert_eq!((next.main_width, next.preview_width), (197.0, 197.0));
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(preview_resize(&previous, previous.frame, invalid, false).is_err());
        }
        assert!(preview_resize(&layout(0.0), previous.frame, 560.0, false).is_err());
    }
    #[test]
    fn reduces_preview_then_sidebar_then_main() {
        assert_eq!(allocate(560.0, 240.0, 700.0, 1200.0), (560.0, 240.0, 388.0));
        assert_eq!(allocate(560.0, 240.0, 700.0, 1100.0), (560.0, 208.0, 320.0));
        assert_eq!(allocate(560.0, 240.0, 700.0, 900.0), (388.0, 180.0, 320.0));
        assert_eq!(allocate(560.0, 0.0, 0.0, 900.0), (560.0, 0.0, 0.0));
    }

    // ---- The small window (docs/specs/popup-inbox-collapse.md) --------------------------------

    const MINI: Mini = Mini {
        width: 260.0,
        height: 180.0,
    };
    /// Width of the Sidebar column plus its gap in the default layout.
    const LEFT: f64 = 246.0;
    const HOME_X: f64 = 600.0;

    /// A window and the geometry owner, with the native steps of a transaction simulated: the
    /// window takes the planned frame, then the page's final layout is published.
    struct Sim {
        geometry: Geometry,
        frame: Frame,
        work: Rect,
        special: bool,
    }

    impl Sim {
        fn new() -> Self {
            Sim {
                geometry: Geometry::default(),
                frame: Frame {
                    x: HOME_X,
                    y: 120.0,
                    width: 560.0,
                    height: 620.0,
                },
                work: Rect {
                    x: 0.0,
                    y: 25.0,
                    width: 1728.0,
                    height: 1000.0,
                },
                special: false,
            }
        }

        fn native(&self) -> Native {
            Native {
                frame: self.frame,
                scale: 2.0,
                work: Some(self.work),
                special: self.special,
            }
        }

        fn defaults() -> Defaults {
            Defaults {
                main: (560.0, 620.0),
                preview: 700.0,
                sidebar: 240.0,
            }
        }

        fn try_prepare(&mut self, request: Request) -> Result<Planned, PlanError> {
            self.geometry
                .plan(self.native(), &request, &Self::defaults())
        }

        /// Prepare, move the window, finish: what `prepare`, `commit_frame` and `finish` do.
        fn run(&mut self, request: Request) -> Allocation {
            let Planned { mut allocation, .. } = self.try_prepare(request).unwrap();
            self.frame = allocation.frame;
            self.geometry.committed(&allocation);
            if !allocation.collapsed {
                allocation.responsive();
            }
            let revision = allocation.revision;
            self.geometry.finished(revision, &allocation);
            allocation
        }

        fn user_move(&mut self, dx: f64) -> bool {
            self.frame.x += dx;
            self.geometry
                .moved_to(self.frame, Some(self.work), self.special)
        }

        /// `popup_inbox_idle`: the window goes home, then the next round starts from nothing.
        fn end_round(&mut self) {
            self.run(Request {
                sidebar: Some(false),
                preview: Some(false),
                form: Form::Expanded,
                home: true,
                ..Request::default()
            });
            let canvas_right = self.geometry.canvas_right;
            self.geometry = Geometry {
                canvas_right,
                ..Geometry::default()
            };
        }
    }

    fn ask(sidebar: bool, form: Form) -> Request {
        Request {
            sidebar: Some(sidebar),
            preview: Some(false),
            form,
            ..Request::default()
        }
    }

    /// Where the pinned viewport starts on screen; window-anchored changes must not move it.
    #[cfg(target_os = "macos")]
    fn viewport_origin(allocation: &Allocation) -> f64 {
        allocation.frame.x + allocation.left_span() - allocation.canvas.as_ref().unwrap().left
    }

    type Round = fn(&mut Sim, f64);

    /// Runs `round` six times, moving the window by `shift` (or back) inside it, and checks that
    /// each round starts exactly where the previous one started plus what the user moved.
    fn assert_rounds_do_not_drift(name: &str, shift: f64, round: Round) {
        let mut sim = Sim::new();
        let mut expected = None;
        for index in 0..6 {
            let moved = if index % 2 == 0 { shift } else { -shift };
            let before = sim.frame.x;
            round(&mut sim, moved);
            if let Some(first) = expected {
                assert_eq!(before, first, "{name}: round {index} started at {before}");
            }
            expected = Some(before + moved);
            assert!(sim.geometry.published.is_none(), "{name}: not reset");
            assert_eq!(sim.geometry.drift, 0.0);
        }
    }

    fn first_window(sim: &mut Sim) -> Allocation {
        sim.run(ask(false, Form::Expanded))
    }

    #[test]
    fn collapse_keeps_the_left_top_corner_and_the_expanded_layout() {
        let mut sim = Sim::new();
        let single = first_window(&mut sim);
        let grown = sim.run(ask(true, Form::Expanded));
        assert_eq!(grown.frame.x, single.frame.x - LEFT);
        assert_eq!(sim.geometry.anchor, Some((single.frame.x, single.frame.y)));
        let small = sim.run(ask(true, Form::Collapsed(MINI)));
        assert!(small.collapsed);
        assert_eq!(small.frame.x, grown.frame.x);
        assert_eq!(small.frame.y, grown.frame.y);
        assert_eq!((small.frame.width, small.frame.height), (260.0, 180.0));
        // The page stays laid out for the expanded window; only the frame is small.
        assert_eq!(small.sidebar_width, grown.sidebar_width);
        assert_eq!(small.main_width, grown.main_width);
        assert_eq!(small.main_height, grown.main_height);
        assert_eq!(small.limited, grown.limited);
        // The body keeps its place on screen across both changes.
        assert_eq!(sim.geometry.anchor, Some((single.frame.x, single.frame.y)));
        let expanded = sim.run(ask(true, Form::Expanded));
        assert!(!expanded.collapsed);
        assert_eq!(expanded.frame.x, grown.frame.x);
        assert_eq!(expanded.frame.y, grown.frame.y);
        assert_eq!(expanded.frame.width, grown.frame.width);
        assert_eq!(expanded.frame.height, grown.frame.height);
    }

    #[test]
    fn the_sidebar_width_is_known_before_a_sidebar_exists() {
        // A single request has no Sidebar, but its small window is the list: the page needs the width
        // it will have to measure the list before the Sidebar column appears.
        let mut sim = Sim::new();
        let single = first_window(&mut sim);
        assert_eq!(single.sidebar_width, 0.0);
        assert_eq!(single.sidebar_preferred, 240.0);
        // The user's choice replaces the default, and survives the Sidebar going away again.
        let wide = sim.run(Request {
            sidebar: Some(true),
            sidebar_width: Some(330.0),
            ..ask(true, Form::Expanded)
        });
        assert_eq!((wide.sidebar_width, wide.sidebar_preferred), (330.0, 330.0));
        let gone = sim.run(ask(false, Form::Expanded));
        assert_eq!((gone.sidebar_width, gone.sidebar_preferred), (0.0, 330.0));
        let small = sim.run(ask(true, Form::Collapsed(MINI)));
        assert_eq!(small.sidebar_width, 330.0);
        assert_eq!(small.sidebar_preferred, 330.0);
    }

    #[test]
    fn form_changes_are_classified() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        for (form, expected) in [
            (Form::Expanded, Transition::Plain),
            (Form::Collapsed(MINI), Transition::Collapse),
            (
                Form::Collapsed(Mini {
                    width: 300.0,
                    height: 240.0,
                }),
                Transition::MiniResize,
            ),
            (Form::Keep, Transition::MiniResize),
            (Form::Expanded, Transition::Expand),
            (Form::Keep, Transition::Plain),
        ] {
            sim.try_prepare(ask(false, form)).unwrap();
            assert_eq!(sim.geometry.transition, Some(expected));
            sim.geometry.pending = false;
            // Settle it the way the owner does so that the next step sees the new form.
            sim.run(ask(false, form));
        }
    }

    #[test]
    fn keep_retains_the_small_window_size() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        sim.run(ask(false, Form::Collapsed(MINI)));
        let kept = sim.run(Request {
            preview: Some(false),
            ..Request::default()
        });
        assert!(kept.collapsed);
        assert_eq!((kept.frame.width, kept.frame.height), (260.0, 180.0));
    }

    #[test]
    fn the_small_window_is_clamped_to_the_work_area() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        let huge = sim.run(ask(
            false,
            Form::Collapsed(Mini {
                width: 9000.0,
                height: 9000.0,
            }),
        ));
        assert_eq!(huge.frame.width, sim.work.width);
        assert_eq!(huge.frame.height, sim.work.height);
        assert_eq!(huge.frame.x, sim.work.x);
        assert_eq!(huge.frame.y, sim.work.y);
        // A window near the right edge stays inside the screen when it is small and when it grows.
        let mut sim = Sim::new();
        first_window(&mut sim);
        sim.frame.x = sim.work.right() - 300.0;
        sim.geometry.moved_to(sim.frame, Some(sim.work), false);
        let edge = sim.run(ask(false, Form::Collapsed(MINI)));
        assert_eq!(edge.frame.x, sim.work.right() - 300.0);
        let back = sim.run(ask(false, Form::Expanded));
        assert_eq!(back.frame.x, sim.work.right() - back.frame.width);
    }

    #[test]
    fn invalid_small_window_requests_are_refused_without_side_effects() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        for (width, height) in [
            (f64::NAN, 100.0),
            (100.0, f64::INFINITY),
            (0.0, 100.0),
            (100.0, 0.5),
            (-10.0, 100.0),
        ] {
            let result = sim.try_prepare(ask(false, Form::Collapsed(Mini { width, height })));
            assert_eq!(
                result.err(),
                Some(PlanError::Invalid("Small window size is invalid"))
            );
            assert!(!sim.geometry.pending);
        }
        // Maximized and full-screen windows keep their size.
        sim.special = true;
        let result = sim.try_prepare(ask(false, Form::Collapsed(MINI)));
        assert!(matches!(result, Err(PlanError::Invalid(_))));
        assert!(!sim.geometry.pending);
        assert!(!sim.geometry.is_collapsed());
    }

    #[test]
    fn a_transaction_in_flight_refuses_the_next_one() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        sim.try_prepare(ask(false, Form::Collapsed(MINI))).unwrap();
        assert!(sim.geometry.pending);
        assert_eq!(
            sim.try_prepare(ask(false, Form::Expanded)).err(),
            Some(PlanError::Busy)
        );
        // A stale revision cannot finish the transaction that is in flight.
        let revision = sim.geometry.revision;
        let prepared = sim.geometry.prepared.clone().unwrap();
        sim.geometry.finished(revision + 1, &prepared);
        assert!(sim.geometry.pending);
        sim.geometry.finished(revision, &prepared);
        assert!(!sim.geometry.pending);
        assert!(sim.geometry.is_collapsed());
    }

    #[test]
    fn resize_events_never_count_as_edge_drags_while_pending_or_small() {
        let mut sim = Sim::new();
        assert!(!sim.geometry.ignores_resize());
        first_window(&mut sim);
        assert!(!sim.geometry.ignores_resize());
        sim.try_prepare(ask(false, Form::Collapsed(MINI))).unwrap();
        assert!(sim.geometry.ignores_resize());
        sim.geometry.pending = false;
        assert!(!sim.geometry.ignores_resize());
        sim.run(ask(false, Form::Collapsed(MINI)));
        assert!(sim.geometry.ignores_resize());
        sim.run(ask(false, Form::Expanded));
        assert!(!sim.geometry.ignores_resize());
    }

    #[test]
    fn the_small_window_remembers_nothing() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        let main = sim.geometry.preferred_main;
        let normal = sim.geometry.normal.map(|f| (f.x, f.y, f.width, f.height));
        let planned = sim
            .try_prepare(Request {
                main_extent: Some(500.0),
                ..ask(false, Form::Collapsed(MINI))
            })
            .unwrap();
        assert!(planned.durable.is_none());
        sim.geometry.pending = false;
        sim.run(ask(false, Form::Collapsed(MINI)));
        assert_eq!(sim.geometry.preferred_main, main);
        assert_eq!(
            sim.geometry.normal.map(|f| (f.x, f.y, f.width, f.height)),
            normal
        );
    }

    #[test]
    fn moving_the_small_window_moves_the_anchor_by_the_expanded_columns() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        sim.run(ask(true, Form::Expanded));
        let small = sim.run(ask(true, Form::Collapsed(MINI)));
        assert_eq!(small.frame.x, HOME_X - LEFT);
        assert!(!sim.user_move(300.0));
        assert_eq!(sim.geometry.anchor.map(|a| a.0), Some(HOME_X + 300.0));
        // Ignored while a transaction runs and for maximized windows.
        sim.try_prepare(ask(true, Form::Expanded)).unwrap();
        sim.user_move(50.0);
        assert_eq!(sim.geometry.anchor.map(|a| a.0), Some(HOME_X + 300.0));
        sim.geometry.pending = false;
        sim.special = true;
        sim.user_move(50.0);
        assert_eq!(sim.geometry.anchor.map(|a| a.0), Some(HOME_X + 300.0));
    }

    #[test]
    fn a_work_area_change_asks_for_one_reconcile() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        let other = Rect {
            width: 1440.0,
            ..sim.work
        };
        assert!(sim.geometry.moved_to(sim.frame, Some(other), false));
        assert!(!sim.geometry.moved_to(sim.frame, Some(sim.work), false));
        sim.geometry.reconcile_queued = false;
        assert!(sim.geometry.moved_to(sim.frame, Some(other), false));
    }

    #[test]
    fn dragging_is_remembered_for_the_prepared_transaction_only() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        sim.try_prepare(Request {
            dragging: true,
            ..ask(false, Form::Collapsed(MINI))
        })
        .unwrap();
        assert!(sim.geometry.dragging);
        sim.geometry.pending = false;
        sim.try_prepare(ask(false, Form::Expanded)).unwrap();
        assert!(!sim.geometry.dragging);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn window_anchored_changes_keep_the_viewport_on_screen() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        let grown = sim.run(ask(true, Form::Expanded));
        let origin = viewport_origin(&grown);
        // Collapse, a change of the left column while small, expand: the pinned viewport never moves,
        // and the body origin only follows the column left of it.
        let small = sim.run(ask(true, Form::Collapsed(MINI)));
        assert_eq!(viewport_origin(&small), origin);
        let shorter = sim.run(ask(false, Form::Collapsed(MINI)));
        assert_eq!(viewport_origin(&shorter), origin);
        assert!(shorter.canvas.as_ref().unwrap().frozen);
        let again = sim.run(ask(true, Form::Collapsed(MINI)));
        assert_eq!(viewport_origin(&again), origin);
        let back = sim.run(ask(false, Form::Expanded));
        assert_eq!(viewport_origin(&back), origin);
        assert!(!back.canvas.as_ref().unwrap().frozen);
        assert_eq!(
            back.canvas.as_ref().unwrap().left,
            grown.canvas.as_ref().unwrap().left - LEFT
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plain_changes_keep_the_body_and_do_not_shift_the_canvas() {
        let mut sim = Sim::new();
        let single = first_window(&mut sim);
        let grown = sim.run(ask(true, Form::Expanded));
        assert_eq!(
            grown.canvas.as_ref().unwrap().left,
            single.canvas.as_ref().unwrap().left
        );
        assert_eq!(
            sim.geometry.anchor.map(|a| a.0),
            Some(single.frame.x + single.left_span())
        );
    }

    #[test]
    fn rounds_that_grow_collapse_and_expand_start_where_they_ended() {
        assert_rounds_do_not_drift("grow, collapse, expand", 0.0, |sim, _| {
            first_window(sim);
            sim.run(ask(true, Form::Expanded));
            sim.run(ask(true, Form::Collapsed(MINI)));
            sim.run(ask(true, Form::Expanded));
            sim.end_round();
        });
    }

    #[test]
    fn rounds_keep_where_the_user_moved_the_small_window() {
        assert_rounds_do_not_drift("collapse, user move, expand", 300.0, |sim, shift| {
            first_window(sim);
            sim.run(ask(true, Form::Expanded));
            sim.run(ask(true, Form::Collapsed(MINI)));
            sim.user_move(shift);
            sim.run(ask(true, Form::Expanded));
            sim.end_round();
        });
    }

    #[test]
    fn rounds_where_a_request_arrives_at_the_small_window_start_where_they_ended() {
        assert_rounds_do_not_drift("single collapse, arrival expands", 0.0, |sim, _| {
            first_window(sim);
            sim.run(ask(false, Form::Collapsed(MINI)));
            // The second request is a primary one: Sidebar appears and the window expands.
            let arrived = sim.run(ask(true, Form::Expanded));
            assert_eq!(arrived.frame.x, HOME_X);
            assert_eq!(sim.geometry.drift, LEFT);
            sim.end_round();
        });
    }

    #[test]
    fn rounds_where_a_request_ends_while_small_start_where_they_ended() {
        assert_rounds_do_not_drift("pair collapse, one ends, expand", 0.0, |sim, _| {
            first_window(sim);
            sim.run(ask(true, Form::Expanded));
            sim.run(ask(true, Form::Collapsed(MINI)));
            // One request is gone: the list is shorter and the Sidebar column disappears.
            let shorter = sim.run(ask(false, Form::Collapsed(MINI)));
            assert_eq!(shorter.frame.x, HOME_X - LEFT);
            assert_eq!(sim.geometry.drift, -LEFT);
            sim.run(ask(false, Form::Expanded));
            sim.end_round();
        });
    }

    #[test]
    fn rounds_that_end_while_small_start_where_they_ended() {
        assert_rounds_do_not_drift("end while small", 0.0, |sim, _| {
            first_window(sim);
            sim.run(ask(true, Form::Expanded));
            sim.run(ask(true, Form::Collapsed(MINI)));
            sim.end_round();
        });
    }

    #[test]
    fn a_round_that_never_collapses_still_returns_to_its_start() {
        assert_rounds_do_not_drift("sidebar only", 0.0, |sim, _| {
            first_window(sim);
            sim.run(ask(true, Form::Expanded));
            sim.run(ask(false, Form::Expanded));
            sim.run(ask(true, Form::Expanded));
            sim.end_round();
        });
    }

    #[test]
    fn going_home_from_the_small_window_expands_it_without_a_sidebar() {
        let mut sim = Sim::new();
        first_window(&mut sim);
        sim.run(ask(true, Form::Expanded));
        sim.run(ask(true, Form::Collapsed(MINI)));
        sim.user_move(120.0);
        let home = sim.run(Request {
            sidebar: Some(false),
            preview: Some(false),
            form: Form::Expanded,
            home: true,
            ..Request::default()
        });
        assert!(!home.collapsed);
        assert_eq!(sim.geometry.transition, None);
        assert_eq!(home.sidebar_width, 0.0);
        assert_eq!(home.frame.x, HOME_X + 120.0);
    }
}
