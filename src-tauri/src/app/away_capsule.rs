//! The desktop capsule of away mode and the "welcome back" reminder
//! (`docs/specs/special-replies.md` D22, D23, N7; picture: `docs/assets/special-replies/mac-hud.jpg`).
//!
//! While away mode is on, the GUI Host keeps one small window above everything else, on every
//! Space, that never takes keyboard focus: a capsule saying how long the user has been away, which
//! grows into a card on hover or click. When the Mac has been idle for a while and the user comes
//! back, the same card turns into "welcome back". The window is macOS only (no idle time and no
//! comparable window behaviour elsewhere); everything that decides *what* it shows is plain data
//! and lives at the top of this file so it can be tested anywhere.

use crate::daemon::away::{AwaySnapshot, SessionMode};
use crate::ipc::TrayAgentInfo;
use serde::{Deserialize, Serialize};

/// Label of the capsule window. It is not one of the Host's "hosted" windows: it must neither keep
/// the daemon alive nor stop the Host from exiting.
pub const LABEL: &str = "away-capsule";

/// Idle time after which the next input counts as "coming back" (N7).
pub const WELCOME_IDLE_SECS: u64 = 300;

/// Agent rows the expanded card lists before folding the rest into "and N more".
const MAX_ROWS: usize = 4;

/// One Agent session in the expanded card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsuleRow {
    /// Agent family (`claude` / `codex` / `cursor` / `grok`); the card shows its display name.
    pub kind: String,
    pub title: String,
    pub replies: u32,
}

/// Everything the capsule and its card show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsuleView {
    /// `all` while the global switch is on, `sessions` when only single sessions are away.
    pub scope: String,
    pub since_ms: u64,
    /// How many sessions are away on their own (scope `sessions` only).
    pub sessions: u32,
    /// Their keys, so "end away mode" on the card can put each back to following the global switch.
    pub session_keys: Vec<String>,
    pub replies: u32,
    /// Escalated questions waiting for an answer (the blue counter).
    pub waiting: u32,
    pub rows: Vec<CapsuleRow>,
    /// Rows that did not fit.
    pub more: u32,
    pub note: Option<String>,
}

/// What the Host tells the capsule window: the view (or `None` = nothing to show) and whether the
/// card is the "welcome back" one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsuleState {
    pub view: Option<CapsuleView>,
    pub welcome: bool,
}

/// The capsule exists while the global switch is on or at least one session is away on its own;
/// "stop using AskHuman" is not away mode and gets no capsule.
pub fn view_model(away: &AwaySnapshot, agents: &[TrayAgentInfo]) -> Option<CapsuleView> {
    let row = |key: &str, replies: u32| -> Option<CapsuleRow> {
        let id = key.strip_prefix("sid:")?;
        let agent = agents.iter().find(|a| a.session_id == id)?;
        Some(CapsuleRow {
            kind: agent.kind.clone(),
            title: agent.title.clone(),
            replies,
        })
    };
    let mut view = if let Some(global) = &away.global {
        CapsuleView {
            scope: "all".to_string(),
            since_ms: global.since_ms,
            sessions: 0,
            session_keys: Vec::new(),
            replies: global.replies,
            waiting: 0,
            // Sessions nobody has answered are left out; the total still counts every reply.
            rows: global
                .replies_by_session
                .iter()
                .filter(|(_, n)| **n > 0)
                .filter_map(|(key, n)| row(key, *n))
                .collect(),
            more: 0,
            note: global.note.clone(),
        }
    } else {
        let own: Vec<_> = away
            .sessions
            .iter()
            .filter(|s| s.mode == SessionMode::Away)
            .collect();
        CapsuleView {
            scope: "sessions".to_string(),
            since_ms: own.iter().map(|s| s.since_ms).min()?,
            sessions: own.len() as u32,
            session_keys: own.iter().map(|s| s.session_key.clone()).collect(),
            replies: own.iter().map(|s| s.replies).sum(),
            waiting: 0,
            // Every away session is listed, also one that has not been answered yet: they are
            // what the scope consists of.
            rows: own
                .iter()
                .filter_map(|s| row(&s.session_key, s.replies))
                .collect(),
            more: 0,
            note: None,
        }
    };
    view.waiting = away.escalated_waiting;
    view.rows.sort_by(|a, b| {
        b.replies
            .cmp(&a.replies)
            .then_with(|| a.title.cmp(&b.title))
    });
    view.more = view.rows.len().saturating_sub(MAX_ROWS) as u32;
    view.rows.truncate(MAX_ROWS);
    Some(view)
}

/// Decides when "welcome back" shows: the first input after the Mac had been idle for
/// [`WELCOME_IDLE_SECS`] while away mode was on. It only reminds; it never ends away mode (D23).
#[derive(Debug, Default)]
pub struct WelcomeWatch {
    armed: bool,
}

impl WelcomeWatch {
    /// One idle sample (seconds since the last keyboard or mouse input; `None` = unknown).
    /// Returns true exactly once per idle spell, when the user is back.
    pub fn tick(&mut self, away_on: bool, enabled: bool, idle_secs: Option<u64>) -> bool {
        if !away_on || !enabled {
            self.armed = false;
            return false;
        }
        let Some(idle) = idle_secs else {
            return false;
        };
        if idle >= WELCOME_IDLE_SECS {
            self.armed = true;
            return false;
        }
        if self.armed {
            // Input resumed since the spell began (a sample can be late, so any idle time below
            // the threshold means the user is back).
            self.armed = false;
            return true;
        }
        false
    }
}

/// The top-centre of the capsule window in logical points (the global desktop space macOS itself
/// uses, which stays consistent across displays of different scales); what is remembered when the
/// user drags it. The width changes between the capsule and the card, so the centre is the stable
/// part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub cx: i32,
    pub top: i32,
}

/// A rectangle on the virtual desktop, in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Area {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A monitor rectangle in logical points. Tao reports monitors in physical pixels (the points times
/// that monitor's own scale), so dividing by the same scale gives the points back exactly, whatever
/// the scales of the other displays are.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn area_in_points(
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    scale: f64,
) -> Area {
    Area {
        x: (f64::from(position.x) / scale).round() as i32,
        y: (f64::from(position.y) / scale).round() as i32,
        w: (f64::from(size.width) / scale).round() as i32,
        h: (f64::from(size.height) / scale).round() as i32,
    }
}

/// Where the window's top-left corner goes: at the remembered anchor while it still lies on a
/// screen, otherwise top centre of the main screen's work area (right below the menu bar, which
/// on a notched display is right below the notch).
pub fn place(
    saved: Option<Anchor>,
    screens: &[Area],
    main_work_area: Area,
    width: i32,
    margin: i32,
) -> (i32, i32) {
    let anchor = saved
        .filter(|a| screens.iter().any(|s| s.contains(a.cx, a.top)))
        .unwrap_or(Anchor {
            cx: main_work_area.x + main_work_area.w / 2,
            top: main_work_area.y + margin,
        });
    (anchor.cx - width / 2, anchor.top)
}

fn load_anchor() -> Option<Anchor> {
    let text = std::fs::read_to_string(crate::paths::away_capsule_file()).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_anchor(anchor: Anchor) {
    let path = crate::paths::away_capsule_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(&anchor) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(target_os = "macos")]
pub use window::{dismiss_welcome, focus_waiting, init, moved, resize, start_welcome_watch, sync};

#[cfg(not(target_os = "macos"))]
pub use stub::{dismiss_welcome, focus_waiting, init, moved, resize, start_welcome_watch, sync};

/// What the capsule window asks for when it opens: the state plus the look of the app.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsuleInit {
    pub state: CapsuleState,
    pub theme: String,
    pub lang: String,
}

#[cfg(not(target_os = "macos"))]
mod stub {
    use super::*;
    use tauri::AppHandle;

    pub fn sync(_app: &AppHandle) {}
    pub fn start_welcome_watch(_app: &AppHandle) {}
    pub fn init(_app: &AppHandle) -> CapsuleInit {
        CapsuleInit {
            state: CapsuleState::default(),
            theme: "system".into(),
            lang: "en".into(),
        }
    }
    pub fn resize(_app: &AppHandle, _width: f64, _height: f64) {}
    pub fn dismiss_welcome(_app: &AppHandle) {}
    pub fn focus_waiting(_app: &AppHandle) {}
    pub fn moved(_window: &tauri::Window) {}
}

#[cfg(target_os = "macos")]
mod window {
    use super::*;
    use crate::app::gui_host::HostState;
    use crate::config::{AppConfig, MenuBarIconMode, ThemeMode};
    use crate::ipc::{self, ClientMsg};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    use tauri::{
        AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl,
        WebviewWindowBuilder, Window,
    };

    /// Gap between the menu bar and the capsule.
    const MARGIN_LOGICAL: f64 = 8.0;
    /// The size the window opens at, before the page reports the real one.
    const INITIAL_SIZE: (f64, f64) = (260.0, 44.0);
    /// How often the Mac's idle time is sampled while away mode is on.
    const SAMPLE_EVERY: Duration = Duration::from_secs(3);
    /// Moves within this long after our own resize are not the user dragging.
    const OWN_MOVE_GRACE: Duration = Duration::from_millis(400);
    /// The drag has ended once the window has not moved for this long.
    const DRAG_SETTLE: Duration = Duration::from_millis(500);

    static MOVE_GENERATION: AtomicU64 = AtomicU64::new(0);
    static LAST_OWN_MOVE: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

    fn note_own_move() {
        *LAST_OWN_MOVE.lock().unwrap() = Some(Instant::now());
    }

    fn recently_moved_by_us() -> bool {
        LAST_OWN_MOVE
            .lock()
            .unwrap()
            .is_some_and(|at| at.elapsed() < OWN_MOVE_GRACE)
    }

    /// What the capsule should show right now.
    fn desired_view(app: &AppHandle) -> Option<CapsuleView> {
        let state = app.try_state::<HostState>()?;
        if state.mode() == MenuBarIconMode::Off || !state.daemon_up_now() {
            return None;
        }
        if !state.away_config().capsule {
            return None;
        }
        let data = state.data.lock().unwrap();
        view_model(data.away.as_ref()?, &data.agents)
    }

    /// Whether away mode is on at all (global or any session), the welcome watch's input.
    fn away_on(app: &AppHandle) -> bool {
        app.try_state::<HostState>().is_some_and(|state| {
            state
                .data
                .lock()
                .unwrap()
                .away
                .as_ref()
                .is_some_and(|away| away.any_away())
        })
    }

    /// Bring the window in line with the away state: open it, update it or close it. Runs on the
    /// main thread (called from the tray refresh); cheap when nothing changed.
    pub fn sync(app: &AppHandle) {
        let Some(state) = app.try_state::<HostState>() else {
            return;
        };
        let wanted = desired_view(app);
        let (changed, next) = {
            let mut current = state.capsule.lock().unwrap();
            let welcome = current.welcome && wanted.is_some();
            let next = CapsuleState {
                view: wanted,
                welcome,
            };
            let changed = *current != next;
            *current = next.clone();
            (changed, next)
        };
        let existing = app.get_webview_window(LABEL);
        match (&next.view, existing) {
            (None, Some(window)) => {
                let _ = window.close();
            }
            (Some(_), None) => {
                if let Err(error) = open(app) {
                    eprintln!("away capsule: {error}");
                }
            }
            (Some(_), Some(_)) if changed => {
                let _ = app.emit_to(LABEL, "capsule-state", &next);
            }
            _ => {}
        }
    }

    fn snapshot(app: &AppHandle) -> CapsuleState {
        app.try_state::<HostState>()
            .map(|state| state.capsule.lock().unwrap().clone())
            .unwrap_or_default()
    }

    pub fn init(app: &AppHandle) -> CapsuleInit {
        let config = AppConfig::load_without_secrets();
        CapsuleInit {
            state: snapshot(app),
            theme: match config.general.theme {
                ThemeMode::Light => "light",
                ThemeMode::Dark => "dark",
                ThemeMode::System => "system",
            }
            .to_string(),
            lang: crate::i18n::Lang::resolve(&config.general.language)
                .code()
                .to_string(),
        }
    }

    fn open(app: &AppHandle) -> tauri::Result<()> {
        let (x, y) = initial_position(app).unwrap_or((0, 0));
        let builder = WebviewWindowBuilder::new(
            app,
            LABEL,
            WebviewUrl::App("index.html?view=away-capsule".into()),
        )
        .title("AskHuman")
        .inner_size(INITIAL_SIZE.0, INITIAL_SIZE.1)
        .position(f64::from(x), f64::from(y))
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        // Showing it must not take the keyboard from whatever the user is typing into.
        .focused(false)
        .focusable(false)
        .accept_first_mouse(true);
        let window = builder.build()?;
        note_own_move();
        harden(&window);
        Ok(())
    }

    /// Top-left of the window in logical points: where the user left it, or top centre.
    fn initial_position(app: &AppHandle) -> Option<(i32, i32)> {
        let primary = app.primary_monitor().ok().flatten()?;
        let work = primary.work_area();
        let main_work_area = area_in_points(work.position, work.size, primary.scale_factor());
        let screens: Vec<Area> = app
            .available_monitors()
            .ok()?
            .iter()
            .map(|m| area_in_points(*m.position(), *m.size(), m.scale_factor()))
            .collect();
        Some(place(
            load_anchor(),
            &screens,
            main_work_area,
            INITIAL_SIZE.0.round() as i32,
            MARGIN_LOGICAL.round() as i32,
        ))
    }

    /// Everything a plain window cannot be told: above full-screen apps, on every Space, never hidden
    /// when the app is deactivated, and in front without activating the app.
    fn harden(window: &tauri::WebviewWindow) {
        use objc2::msg_send;
        use objc2::runtime::{AnyObject, Bool};
        /// canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary.
        const BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);
        /// NSStatusWindowLevel: above ordinary windows and the always-on-top popups.
        const STATUS_LEVEL: isize = 25;
        let Ok(pointer) = window.ns_window() else {
            return;
        };
        let ns_window = pointer as *mut AnyObject;
        // SAFETY: the pointer is the live NSWindow of this window and we are on the main thread.
        unsafe {
            let _: () = msg_send![ns_window, setCollectionBehavior: BEHAVIOR];
            let _: () = msg_send![ns_window, setLevel: STATUS_LEVEL];
            let _: () = msg_send![ns_window, setHidesOnDeactivate: Bool::NO];
            let _: () = msg_send![ns_window, orderFrontRegardless];
        }
    }

    /// The page reports its natural size (collapsed capsule or expanded card); the window follows
    /// and keeps its top-centre where it was.
    pub fn resize(app: &AppHandle, width: f64, height: f64) {
        let Some(window) = app.get_webview_window(LABEL) else {
            return;
        };
        if !(width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0) {
            return;
        }
        let (Ok(position), Ok(size), Ok(scale)) = (
            window.outer_position(),
            window.outer_size(),
            window.scale_factor(),
        ) else {
            return;
        };
        // In logical points, like everything that is remembered about the window's place.
        let position = position.to_logical::<f64>(scale);
        let size = size.to_logical::<f64>(scale);
        let centre = position.x + size.width / 2.0;
        note_own_move();
        let _ = window.set_size(LogicalSize::new(width, height));
        let _ = window.set_position(LogicalPosition::new(centre - width / 2.0, position.y));
    }

    /// The user moved the window (or we did, a moment ago): remember where it ended up.
    pub fn moved(window: &Window) {
        if recently_moved_by_us() {
            return;
        }
        let generation = MOVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        let window = window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(DRAG_SETTLE);
            if MOVE_GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            if let (Ok(position), Ok(size), Ok(scale)) = (
                window.outer_position(),
                window.outer_size(),
                window.scale_factor(),
            ) {
                let position = position.to_logical::<f64>(scale);
                let size = size.to_logical::<f64>(scale);
                save_anchor(Anchor {
                    cx: (position.x + size.width / 2.0).round() as i32,
                    top: position.y.round() as i32,
                });
            }
        });
    }

    /// "Keep away": the card goes back to being a capsule.
    pub fn dismiss_welcome(app: &AppHandle) {
        let Some(state) = app.try_state::<HostState>() else {
            return;
        };
        let next = {
            let mut current = state.capsule.lock().unwrap();
            if !current.welcome {
                return;
            }
            current.welcome = false;
            current.clone()
        };
        let _ = app.emit_to(LABEL, "capsule-state", &next);
    }

    /// The card's "view" button: bring the escalated question's popup forward.
    pub fn focus_waiting(app: &AppHandle) {
        let Some(state) = app.try_state::<HostState>() else {
            return;
        };
        let request_id = {
            let data = state.data.lock().unwrap();
            data.pending_requests
                .iter()
                .find(|r| r.escalated)
                .or_else(|| data.pending_requests.first())
                .map(|r| r.id.clone())
        };
        let Some(request_id) = request_id else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            if let Ok(stream) = crate::client::connect_stream().await {
                let (_r, mut w) = stream.into_split();
                let _ = ipc::write_msg(&mut w, &ClientMsg::FocusRequest { request_id }).await;
            }
        });
    }

    /// Samples the Mac's idle time while away mode is on and shows "welcome back" when the user
    /// returns after a long idle spell.
    pub fn start_welcome_watch(app: &AppHandle) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut watch = WelcomeWatch::default();
            loop {
                tokio::time::sleep(SAMPLE_EVERY).await;
                let enabled = app
                    .try_state::<HostState>()
                    .is_some_and(|state| state.away_config().welcome_back);
                let idle = crate::idle::secs_since_last_input();
                if watch.tick(away_on(&app), enabled, idle) {
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || show_welcome(&app2));
                }
            }
        });
    }

    fn show_welcome(app: &AppHandle) {
        let Some(state) = app.try_state::<HostState>() else {
            return;
        };
        let next = {
            let mut current = state.capsule.lock().unwrap();
            if current.view.is_none() {
                return;
            }
            current.welcome = true;
            current.clone()
        };
        let _ = app.emit_to(LABEL, "capsule-state", &next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::away::{GlobalView, SessionView};
    use std::collections::BTreeMap;

    fn agent(id: &str, kind: &str, title: &str) -> TrayAgentInfo {
        TrayAgentInfo {
            session_id: id.to_string(),
            seq: 1,
            kind: kind.to_string(),
            title: title.to_string(),
            forked_from_seq: None,
            project_name: String::new(),
            cwd: None,
            state: "working".to_string(),
            pending_interject: false,
            focusable: false,
            forkable: false,
            pid: None,
            launch_id: None,
            mode: None,
        }
    }

    fn global(replies: &[(&str, u32)], total: u32) -> AwaySnapshot {
        AwaySnapshot {
            global: Some(GlobalView {
                since_ms: 1_000,
                note: Some("not prod".into()),
                replies: total,
                replies_by_session: replies
                    .iter()
                    .map(|(k, n)| (k.to_string(), *n))
                    .collect::<BTreeMap<_, _>>(),
            }),
            sessions: Vec::new(),
            escalated_waiting: 0,
        }
    }

    fn own(key: &str, mode: SessionMode, since_ms: u64, replies: u32) -> SessionView {
        SessionView {
            session_key: key.to_string(),
            mode,
            since_ms,
            note: None,
            replies,
        }
    }

    #[test]
    fn nothing_away_means_no_capsule() {
        assert_eq!(view_model(&AwaySnapshot::default(), &[]), None);
        // "Stop using AskHuman" is not away mode.
        let muted = AwaySnapshot {
            sessions: vec![own("sid:a", SessionMode::Mute, 5, 0)],
            ..AwaySnapshot::default()
        };
        assert_eq!(view_model(&muted, &[]), None);
    }

    #[test]
    fn global_away_lists_the_busiest_sessions_first() {
        let agents = [
            agent("a", "codex", "Fix login"),
            agent("b", "claude", "Refactor storage"),
            agent("c", "cursor", "Quiet"),
        ];
        let away = AwaySnapshot {
            escalated_waiting: 1,
            ..global(&[("sid:a", 4), ("sid:b", 2), ("sid:c", 0), ("mcp:x", 3)], 9)
        };
        let view = view_model(&away, &agents).unwrap();
        assert_eq!(view.scope, "all");
        assert!(view.session_keys.is_empty());
        assert_eq!(view.since_ms, 1_000);
        assert_eq!(view.replies, 9);
        assert_eq!(view.waiting, 1);
        assert_eq!(view.note.as_deref(), Some("not prod"));
        // The session nobody answered and the one without a known Agent are not listed, but the
        // total still counts every reply.
        assert_eq!(
            view.rows,
            vec![
                CapsuleRow {
                    kind: "codex".into(),
                    title: "Fix login".into(),
                    replies: 4
                },
                CapsuleRow {
                    kind: "claude".into(),
                    title: "Refactor storage".into(),
                    replies: 2
                },
            ]
        );
        assert_eq!(view.more, 0);
    }

    #[test]
    fn rows_beyond_the_card_height_are_folded() {
        let agents: Vec<_> = (0..6)
            .map(|i| agent(&format!("s{i}"), "codex", &format!("Task {i}")))
            .collect();
        let replies: Vec<(String, u32)> = (0..6).map(|i| (format!("sid:s{i}"), 6 - i)).collect();
        let borrowed: Vec<(&str, u32)> = replies.iter().map(|(k, n)| (k.as_str(), *n)).collect();
        let view = view_model(&global(&borrowed, 21), &agents).unwrap();
        assert_eq!(view.rows.len(), MAX_ROWS);
        assert_eq!(view.more, 2);
        assert_eq!(view.rows[0].title, "Task 0");
    }

    #[test]
    fn a_session_that_is_away_on_its_own_gets_a_session_scoped_capsule() {
        let agents = [
            agent("a", "codex", "Fix login"),
            agent("b", "claude", "Docs"),
        ];
        let away = AwaySnapshot {
            sessions: vec![
                own("sid:a", SessionMode::Away, 700, 2),
                own("sid:b", SessionMode::Away, 400, 0),
                own("sid:c", SessionMode::Mute, 100, 0),
            ],
            escalated_waiting: 2,
            ..AwaySnapshot::default()
        };
        let view = view_model(&away, &agents).unwrap();
        assert_eq!(view.scope, "sessions");
        assert_eq!(view.sessions, 2);
        assert_eq!(view.session_keys, vec!["sid:a", "sid:b"]);
        assert_eq!(view.since_ms, 400);
        assert_eq!(view.replies, 2);
        assert_eq!(view.waiting, 2);
        // Every away session is listed, also the one that has not been answered yet.
        assert_eq!(view.rows.len(), 2);
        assert_eq!(view.rows[0].title, "Fix login");
        assert_eq!(view.rows[1].replies, 0);
    }

    #[test]
    fn the_global_switch_wins_over_single_sessions() {
        let mut away = global(&[], 0);
        away.sessions = vec![own("sid:a", SessionMode::Away, 1, 5)];
        let view = view_model(&away, &[agent("a", "codex", "T")]).unwrap();
        assert_eq!(view.scope, "all");
        assert_eq!(view.replies, 0);
    }

    #[test]
    fn welcome_back_shows_once_after_a_long_idle_spell() {
        let mut watch = WelcomeWatch::default();
        // Busy at the Mac: nothing.
        assert!(!watch.tick(true, true, Some(2)));
        // Idle, but not long enough.
        assert!(!watch.tick(true, true, Some(120)));
        assert!(!watch.tick(true, true, Some(299)));
        // The user is back before the threshold was reached: still nothing.
        assert!(!watch.tick(true, true, Some(3)));
        // A long idle spell, then the first input.
        assert!(!watch.tick(true, true, Some(300)));
        assert!(!watch.tick(true, true, Some(900)));
        assert!(watch.tick(true, true, Some(1)));
        // Once per spell.
        assert!(!watch.tick(true, true, Some(2)));
        assert!(!watch.tick(true, true, Some(40)));
    }

    #[test]
    fn welcome_back_needs_away_mode_and_the_setting() {
        let mut watch = WelcomeWatch::default();
        assert!(!watch.tick(true, true, Some(600)));
        // Away mode ended while the user was away: the spell does not carry over.
        assert!(!watch.tick(false, true, Some(1)));
        assert!(!watch.tick(true, true, Some(1)));
        // The setting is off.
        assert!(!watch.tick(true, false, Some(600)));
        assert!(!watch.tick(true, false, Some(1)));
        assert!(!watch.tick(true, true, Some(1)));
        // An unknown idle time never fires.
        assert!(!watch.tick(true, true, Some(600)));
        assert!(!watch.tick(true, true, None));
        assert!(watch.tick(true, true, Some(1)));
    }

    #[test]
    fn the_window_goes_top_centre_unless_a_remembered_spot_is_still_on_a_screen() {
        // A MacBook screen (1512 x 982 points, menu bar 37 points tall because of the notch) with an
        // external display to its right.
        let main = Area {
            x: 0,
            y: 37,
            w: 1512,
            h: 945,
        };
        let screens = [
            Area {
                x: 0,
                y: 0,
                w: 1512,
                h: 982,
            },
            Area {
                x: 1512,
                y: 0,
                w: 2560,
                h: 1440,
            },
        ];
        // Default: centred below the menu bar.
        assert_eq!(place(None, &screens, main, 260, 8), (626, 45));
        // A spot on the second screen is kept; the window is centred on it.
        let saved = Anchor { cx: 2800, top: 300 };
        assert_eq!(place(Some(saved), &screens, main, 260, 8), (2670, 300));
        // A spot on a screen that is gone falls back to the default.
        let gone = Anchor { cx: 7000, top: 300 };
        assert_eq!(place(Some(gone), &screens, main, 260, 8), (626, 45));
    }

    #[test]
    fn monitors_come_back_as_points_whatever_their_scale() {
        // A 2x display: tao says 6016 x 3324 pixels starting 60 pixels down; macOS says 3008 x 1662
        // points starting 30 down. Treating the pixels as points put the capsule at the far right.
        let area = area_in_points(
            tauri::PhysicalPosition::new(0, 60),
            tauri::PhysicalSize::new(6016, 3324),
            2.0,
        );
        assert_eq!(
            area,
            Area {
                x: 0,
                y: 30,
                w: 3008,
                h: 1662
            }
        );
        // A 1x display to the right keeps its own origin.
        let second = area_in_points(
            tauri::PhysicalPosition::new(3008, 0),
            tauri::PhysicalSize::new(1920, 1080),
            1.0,
        );
        assert_eq!(second.x, 3008);
        assert_eq!((second.w, second.h), (1920, 1080));
    }

    #[test]
    fn a_remembered_spot_survives_a_round_trip_through_json() {
        let anchor = Anchor { cx: 1512, top: 74 };
        let text = serde_json::to_string(&anchor).unwrap();
        assert_eq!(serde_json::from_str::<Anchor>(&text).unwrap(), anchor);
    }
}
