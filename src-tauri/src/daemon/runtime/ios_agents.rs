//! What the iOS gateway sees of the Agent registry: the list, one session's live frame and the
//! interjection queue (docs/plans/ios-channel.md §4.4, §4.7).
//!
//! Everything here reads or writes the same state the Mac's own surfaces use (the status window,
//! IM `/status`, `/watch` and `/msg`); the gateway never reaches into it directly.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::inbound::find_agent_by_session;
use super::watch::frame_detail_json;
use super::*;
use crate::agents::session_info::SessionInfo;
use crate::agents::LifecycleEvent;
use crate::ios::gateway::{AgentEventKind, FrameData, InterjectResult, StagedFile};
use crate::ios::wire::{AgentView, CanInterject, ProjectRef};
use crate::models::FileAttachment;
use crate::todo_sync::identity::{FsResolver, IdentityResolver};

/// Longest "current tool" caption sent to the phone.
const TOOL_CAPTION_MAX_CHARS: usize = 80;

/// The identifier a phone uses for one Agent session.
pub(super) fn agent_key(kind: AgentKind, session_id: &str) -> String {
    format!("{}:{session_id}", kind.as_str())
}

/// Splits `{kind}:{sessionId}`; the session id may contain colons itself.
pub(super) fn split_key(key: &str) -> Option<(AgentKind, &str)> {
    let (kind, session_id) = key.split_once(':')?;
    let kind = AgentKind::parse(kind)?;
    (!session_id.is_empty()).then_some((kind, session_id))
}

pub(super) fn event_kind(event: LifecycleEvent) -> AgentEventKind {
    match event {
        LifecycleEvent::TurnStart => AgentEventKind::TurnStart,
        LifecycleEvent::TurnEnd => AgentEventKind::TurnEnd,
        LifecycleEvent::SessionEnd => AgentEventKind::SessionEnd,
        // A session that just started is a sign of life like any tool call.
        LifecycleEvent::SessionStart | LifecycleEvent::Activity => AgentEventKind::Activity,
    }
}

/// How long a resolved project is trusted. The list is rebuilt every second, but a repository's remote can
/// change under a running Agent and todo sync can be switched on or off, so the answer expires.
const PROJECT_TTL: Duration = Duration::from_secs(60);

/// Resolved project of a working directory, remembered because the list is rebuilt every second
/// and walking up to the git root touches the disk.
#[derive(Default)]
pub(super) struct ProjectCache {
    by_cwd: Mutex<HashMap<String, (ProjectRef, Instant)>>,
}

impl ProjectCache {
    fn of(&self, cwd: &str) -> ProjectRef {
        if cwd.is_empty() {
            return ProjectRef {
                key: String::new(),
                name: String::new(),
                repo: None,
            };
        }
        let mut map = self.by_cwd.lock().unwrap();
        if map.len() > 256 {
            map.clear();
        }
        if let Some((project, at)) = map.get(cwd) {
            if at.elapsed() < PROJECT_TTL {
                return project.clone();
            }
        }
        let project = resolve_project(cwd, crate::todo_sync::service::is_enabled());
        map.insert(cwd.to_string(), (project.clone(), Instant::now()));
        project
    }
}

/// The project a working directory belongs to. `with_repo` (todo sync is on) adds the repository the
/// Todos tab joins on; without sync no phone can use it, and nothing is read for it.
fn resolve_project(cwd: &str, with_repo: bool) -> ProjectRef {
    let key = crate::project::detect_from(Path::new(cwd));
    let repo = if with_repo {
        FsResolver.resolve(&key, None, 0).p
    } else {
        None
    };
    ProjectRef {
        name: crate::project::display_name(&key),
        key,
        repo,
    }
}

fn text<'a>(rec: &'a Value, key: &str) -> &'a str {
    rec.get(key).and_then(Value::as_str).unwrap_or("")
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Whether text can still be handed to this session, and if not, why. The same rules as IM
/// `/msg`: only a working session of a family with a hook that can carry the text.
pub(super) fn can_interject(rec: &Value) -> CanInterject {
    let refuse = |reason: &str| CanInterject {
        ok: false,
        reason: Some(reason.to_string()),
    };
    if text(rec, "state") == "ended" {
        return refuse("ended");
    }
    // Grok has no channel for it; Codex App sessions are driven through the app, which the
    // prototype does not reach from the phone.
    if text(rec, "kind") == "grok" || rec["desktop"]["connected"] == true {
        return refuse("unsupported");
    }
    if text(rec, "state") != "working" {
        return refuse("notWorking");
    }
    CanInterject {
        ok: true,
        reason: None,
    }
}

/// One registry record as the phone shows it. `pending` is the number of queued interjections.
pub(super) fn agent_view(rec: &Value, pending: u64, projects: &ProjectCache) -> Option<AgentView> {
    let kind = AgentKind::parse(text(rec, "kind"))?;
    let session_id = text(rec, "sessionId");
    if session_id.is_empty() {
        return None;
    }
    let waiting_request_id = rec
        .get("waitingRequestId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let state = match text(rec, "state") {
        "ended" => "ended",
        _ if waiting_request_id.is_some() => "waiting",
        "idle" => "idle",
        _ => "working",
    };
    let secs = |key: &str| rec.get(key).and_then(Value::as_u64).unwrap_or(0);
    let current_tool = rec.get("currentTool").and_then(|tool| {
        let name = tool.get("name").and_then(Value::as_str)?.trim();
        if name.is_empty() {
            return None;
        }
        let object = tool
            .get("object")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|object| !object.is_empty());
        Some(clip(
            &match object {
                Some(object) => format!("{name}: {object}"),
                None => name.to_string(),
            },
            TOOL_CAPTION_MAX_CHARS,
        ))
    });
    Some(AgentView {
        key: agent_key(kind, session_id),
        kind: kind.as_str().to_string(),
        kind_label: kind.label().to_string(),
        session_id: session_id.to_string(),
        title: text(rec, "title").trim().to_string(),
        project: projects.of(text(rec, "cwd")),
        state: state.to_string(),
        started_at: secs("startedAt") as i64 * 1000,
        last_activity_at: secs("lastActivity") as i64 * 1000,
        active_elapsed_secs: secs("activeElapsedSecs"),
        turn_steps: secs("turnSteps"),
        current_tool,
        waiting_preview: waiting_request_id
            .as_ref()
            .and(rec.get("waitingPreview"))
            .and_then(Value::as_str)
            .filter(|preview| !preview.is_empty())
            .map(str::to_string),
        waiting_request_id,
        pending_interject: pending,
        last_turn_failed: false,
        can_interject: can_interject(rec),
        followed: false,
    })
}

/// Every session the registry knows, as the phone's list shows them.
pub(super) fn agent_views(state: &Arc<ServerState>, projects: &ProjectCache) -> Vec<AgentView> {
    let snapshot = agents_snapshot_for_gui(state);
    let Some(records) = snapshot.as_array() else {
        return Vec::new();
    };
    records
        .iter()
        .filter_map(|rec| {
            let pending = state
                .interject
                .pending_count(text(rec, "sessionId"))
                .try_into()
                .unwrap_or(u64::MAX);
            agent_view(rec, pending, projects)
        })
        .collect()
}

/// The live frame of one session, or `None` when the registry no longer knows it.
pub(super) fn watch_frame(state: &Arc<ServerState>, key: &str) -> Option<FrameData> {
    let (kind, session_id) = split_key(key)?;
    let snapshot = state.agents.snapshot();
    let rec = find_agent_by_session(&snapshot, session_id)?;
    // The session id alone is enough to find the record: make sure it is the same family.
    if key != agent_key(AgentKind::parse(text(rec, "kind"))?, session_id) {
        return None;
    }
    let seq = rec.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let waiting = state
        .registry
        .in_flight_agent_session_ids()
        .iter()
        .any(|id| id == session_id);
    let frame = crate::watch::build_frame(seq, Some(rec), waiting);
    // The model is read from the session's own files (not from the registry), at most every ten
    // seconds. It is left out of the signature on purpose: a change shows up with the next frame
    // that goes out for any other reason.
    let session = crate::agents::session_info::resolve_cached(kind, session_id);
    Some(FrameData {
        sig: crate::watch::signature(&frame),
        ended: frame.phase == crate::watch::WatchPhase::Ended,
        frame: with_session_model(frame_detail_json(session_id, &frame), &session),
    })
}

/// Longest model name / reasoning level that goes to the phone.
const SESSION_MODEL_MAX_CHARS: usize = 80;
const SESSION_REASONING_MAX_CHARS: usize = 32;

/// Adds `model` and `reasoning` to a frame, each only when the session reported one. The frame is
/// the one the Mac's own console shows; these two keys exist for the phone alone.
fn with_session_model(mut frame: Value, session: &SessionInfo) -> Value {
    use crate::agents::model_catalog::clean_text;
    if let Some(object) = frame.as_object_mut() {
        let fields = [
            ("model", &session.model, SESSION_MODEL_MAX_CHARS),
            ("reasoning", &session.reasoning, SESSION_REASONING_MAX_CHARS),
        ];
        for (name, value, max) in fields {
            let cleaned = value.as_deref().map(|v| clean_text(v, max));
            if let Some(cleaned) = cleaned.filter(|v| !v.is_empty()) {
                object.insert(name.to_string(), Value::String(cleaned));
            }
        }
    }
    frame
}

/// Queues text (and files) for a working session, or hands it to a hook that is waiting right
/// now. Mirrors IM `/msg`: the iOS phone is registered for the "read" receipt of a queued entry.
pub(super) fn interject(
    state: &Arc<ServerState>,
    phone_id: &str,
    agent_key: &str,
    text_in: &str,
    files: Vec<StagedFile>,
) -> InterjectResult {
    let Some((_, session_id)) = split_key(agent_key) else {
        return InterjectResult::rejected("notFound");
    };
    let snapshot = state.agents.snapshot();
    let Some(rec) = find_agent_by_session(&snapshot, session_id) else {
        return InterjectResult::rejected("notFound");
    };
    let can = can_interject(rec);
    if !can.ok {
        return InterjectResult::rejected(can.reason.as_deref().unwrap_or("notWorking"));
    }
    let attachments = files
        .into_iter()
        .map(|file| FileAttachment {
            path: file.path.to_string_lossy().to_string(),
            name: file.name,
            size: file.size,
            is_image: file.is_image,
        })
        .collect();
    let receipt = format!("ios:{phone_id}");
    let queued = state
        .interject
        .append(session_id, text_in, attachments, Some(&receipt));
    state.interject.persist();
    broadcast_agents_state(state);
    if queued == 0 {
        InterjectResult {
            status: "delivered",
            reason: None,
            pending: 0,
        }
    } else {
        InterjectResult {
            status: "queued",
            reason: None,
            pending: queued as u64,
        }
    }
}

/// Withdraws what is queued for a session; `true` when there was something.
pub(super) fn interject_clear(state: &Arc<ServerState>, agent_key: &str) -> bool {
    let Some((_, session_id)) = split_key(agent_key) else {
        return false;
    };
    let had = state.interject.clear(session_id);
    if had {
        state.interject.persist();
        broadcast_agents_state(state);
    }
    had
}

/// `(queued entries, their full text)` for a session.
pub(super) fn interject_state(state: &Arc<ServerState>, agent_key: &str) -> (u64, String) {
    let Some((_, session_id)) = split_key(agent_key) else {
        return (0, String::new());
    };
    (
        state.interject.pending_count(session_id) as u64,
        state.interject.full_text(session_id),
    )
}

/// The phone an `ios:<phoneId>` receipt channel names.
pub(super) fn receipt_phone(channel: &str) -> Option<&str> {
    channel.strip_prefix("ios:").filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(extra: Value) -> Value {
        let mut rec = json!({
            "seq": 3,
            "kind": "claude",
            "sessionId": "s-1",
            "state": "working",
            "title": "  Refactor idle exit ",
            "cwd": "/tmp",
            "startedAt": 1_700_000_000u64,
            "lastActivity": 1_700_000_100u64,
            "activeElapsedSecs": 42,
            "turnSteps": 5,
        });
        if let (Some(base), Some(extra)) = (rec.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                base.insert(key.clone(), value.clone());
            }
        }
        rec
    }

    #[test]
    fn keys_round_trip_and_survive_colons_in_session_ids() {
        assert_eq!(agent_key(AgentKind::Codex, "a:b"), "codex:a:b");
        let (kind, session) = split_key("codex:a:b").unwrap();
        assert_eq!((kind, session), (AgentKind::Codex, "a:b"));
        assert!(split_key("nokind").is_none());
        assert!(split_key("unknown:s1").is_none());
        assert!(split_key("claude:").is_none());
    }

    #[test]
    fn a_working_record_becomes_a_view() {
        let projects = ProjectCache::default();
        let view = agent_view(
            &record(json!({"currentTool": {"name": "Shell", "object": "cargo test", "at": 1}})),
            2,
            &projects,
        )
        .unwrap();
        assert_eq!(view.key, "claude:s-1");
        assert_eq!(view.kind_label, "Claude Code");
        assert_eq!(view.title, "Refactor idle exit");
        assert_eq!(view.state, "working");
        assert_eq!(view.started_at, 1_700_000_000_000);
        assert_eq!(view.last_activity_at, 1_700_000_100_000);
        assert_eq!(view.active_elapsed_secs, 42);
        assert_eq!(view.turn_steps, 5);
        assert_eq!(view.current_tool.as_deref(), Some("Shell: cargo test"));
        assert_eq!(view.pending_interject, 2);
        assert!(view.can_interject.ok);
        assert!(!view.followed && !view.last_turn_failed);
    }

    #[test]
    fn a_session_with_an_open_question_is_waiting() {
        let projects = ProjectCache::default();
        let view = agent_view(
            &record(json!({"waitingRequestId": "r9", "waitingPreview": "Delete it?"})),
            0,
            &projects,
        )
        .unwrap();
        assert_eq!(view.state, "waiting");
        assert_eq!(view.waiting_request_id.as_deref(), Some("r9"));
        assert_eq!(view.waiting_preview.as_deref(), Some("Delete it?"));
        // An ended session has nothing to wait for.
        let view = agent_view(
            &record(json!({"state": "ended", "waitingRequestId": "r9"})),
            0,
            &projects,
        )
        .unwrap();
        assert_eq!(view.state, "ended");
    }

    #[test]
    fn records_without_identity_are_skipped() {
        let projects = ProjectCache::default();
        assert!(agent_view(&record(json!({"sessionId": ""})), 0, &projects).is_none());
        assert!(agent_view(&record(json!({"kind": "mystery"})), 0, &projects).is_none());
    }

    #[test]
    fn interjection_rules_match_im_msg() {
        assert!(can_interject(&record(json!({}))).ok);
        let refused = |extra: Value| can_interject(&record(extra)).reason;
        assert_eq!(
            refused(json!({"state": "idle"})).as_deref(),
            Some("notWorking")
        );
        assert_eq!(refused(json!({"state": "ended"})).as_deref(), Some("ended"));
        assert_eq!(
            refused(json!({"kind": "grok"})).as_deref(),
            Some("unsupported")
        );
        assert_eq!(
            refused(json!({"desktop": {"connected": true}})).as_deref(),
            Some("unsupported")
        );
    }

    #[test]
    fn long_tool_captions_are_clipped() {
        let projects = ProjectCache::default();
        let view = agent_view(
            &record(json!({"currentTool": {"name": "Shell", "object": "x".repeat(300)}})),
            0,
            &projects,
        )
        .unwrap();
        let caption = view.current_tool.unwrap();
        assert_eq!(caption.chars().count(), TOOL_CAPTION_MAX_CHARS);
        assert!(caption.ends_with('…'));
    }

    fn reported(model: Option<&str>, reasoning: Option<&str>) -> SessionInfo {
        SessionInfo {
            model: model.map(str::to_string),
            reasoning: reasoning.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn a_frame_carries_the_model_a_session_reported() {
        let frame = json!({"type": "watchFrame", "seq": 4});
        let both = with_session_model(frame.clone(), &reported(Some("model-a"), Some("high")));
        assert_eq!(both["model"], "model-a");
        assert_eq!(both["reasoning"], "high");
        // Everything else in the frame stays as the Mac's console has it.
        assert_eq!(both["type"], "watchFrame");
        assert_eq!(both["seq"], 4);
        let only_model = with_session_model(frame.clone(), &reported(Some("model-a"), None));
        assert_eq!(only_model["model"], "model-a");
        assert!(only_model.get("reasoning").is_none());
        // Nothing reported: the keys are absent, not empty.
        for session in [
            reported(None, None),
            reported(Some(""), Some("  ")),
            reported(Some("\u{200b}"), None),
        ] {
            let bare = with_session_model(frame.clone(), &session);
            assert_eq!(bare, frame);
        }
    }

    #[test]
    fn what_a_session_reports_is_cleaned_and_clipped_before_it_goes_out() {
        let long = format!("  model\u{200b}-{}  ", "x".repeat(300));
        let frame = with_session_model(json!({}), &reported(Some(&long), Some(&long)));
        let model = frame["model"].as_str().unwrap();
        assert_eq!(model.chars().count(), SESSION_MODEL_MAX_CHARS);
        assert!(model.starts_with("model-x") && model.ends_with('…'));
        let reasoning = frame["reasoning"].as_str().unwrap();
        assert_eq!(reasoning.chars().count(), SESSION_REASONING_MAX_CHARS);
        // A frame that is not an object is passed through untouched.
        let odd = with_session_model(json!([1]), &reported(Some("model-a"), None));
        assert_eq!(odd, json!([1]));
    }

    #[test]
    fn receipt_channels_name_their_phone() {
        assert_eq!(receipt_phone("ios:abc123"), Some("abc123"));
        assert_eq!(receipt_phone("ios:"), None);
        assert_eq!(receipt_phone("feishu"), None);
    }

    fn repo_with_remote(root: &Path, url: &str) {
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(
            root.join(".git/config"),
            format!("[remote \"origin\"]\n\turl = {url}\n"),
        )
        .unwrap();
    }

    #[test]
    fn a_project_names_its_repository_while_todo_sync_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app");
        repo_with_remote(&root, "git@github.com:Acme/App.git");
        let inner = root.join("src/deep");
        std::fs::create_dir_all(&inner).unwrap();

        let project = resolve_project(inner.to_str().unwrap(), true);
        assert_eq!(
            project.key,
            std::fs::canonicalize(&root).unwrap().to_string_lossy()
        );
        assert_eq!(project.name, "app");
        assert_eq!(project.repo.as_deref(), Some("github.com/acme/app"));

        // Nothing is read for a phone that cannot use it.
        assert_eq!(resolve_project(inner.to_str().unwrap(), false).repo, None);
    }

    #[test]
    fn a_worktree_names_the_repository_of_its_main_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("app");
        repo_with_remote(&main, "https://github.com/acme/app");
        let git_dir = main.join(".git/worktrees/feature");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("commondir"), "../..\n").unwrap();
        let tree = dir.path().join("app-feature");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(
            tree.join(".git"),
            format!("gitdir: {}\n", git_dir.to_string_lossy()),
        )
        .unwrap();

        let project = resolve_project(tree.to_str().unwrap(), true);
        assert_eq!(
            project.name, "app-feature",
            "the worktree is its own project here"
        );
        assert_eq!(project.repo.as_deref(), Some("github.com/acme/app"));
    }

    #[test]
    fn a_directory_without_a_remote_has_no_repository() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("scratch");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(resolve_project(plain.to_str().unwrap(), true).repo, None);

        let local = dir.path().join("local");
        repo_with_remote(&local, "/srv/git/local.git");
        assert_eq!(resolve_project(local.to_str().unwrap(), true).repo, None);
    }

    #[test]
    fn a_repository_is_left_out_of_the_wire_when_there_is_none() {
        let bare = ProjectRef {
            key: "/p".into(),
            name: "p".into(),
            repo: None,
        };
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"key": "/p", "name": "p"})
        );
        // Older Macs never send it.
        let parsed: ProjectRef = serde_json::from_value(json!({"key": "/p", "name": "p"})).unwrap();
        assert_eq!(parsed, bare);
        let named = ProjectRef {
            repo: Some("github.com/acme/app".into()),
            ..bare
        };
        assert_eq!(
            serde_json::to_value(&named).unwrap(),
            json!({"key": "/p", "name": "p", "repo": "github.com/acme/app"})
        );
    }

    #[test]
    fn lifecycle_events_map_onto_what_follow_needs() {
        assert_eq!(event_kind(LifecycleEvent::TurnEnd), AgentEventKind::TurnEnd);
        assert_eq!(
            event_kind(LifecycleEvent::TurnStart),
            AgentEventKind::TurnStart
        );
        assert_eq!(
            event_kind(LifecycleEvent::SessionEnd),
            AgentEventKind::SessionEnd
        );
        assert_eq!(
            event_kind(LifecycleEvent::SessionStart),
            AgentEventKind::Activity
        );
        assert_eq!(
            event_kind(LifecycleEvent::Activity),
            AgentEventKind::Activity
        );
    }
}
