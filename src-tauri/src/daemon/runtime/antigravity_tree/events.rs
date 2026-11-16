//! Antigravity hook messages as the daemon handles them: what a conversation's events do to the
//! registry, how the `Stop` gate answers, and when a queued message reaches the agent.
//!
//! The messages are the ones the real hook builds (`antigravity_hook::wire`), played over an
//! in-memory connection into the daemon's own control loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncWriteExt, BufReader, DuplexStream};

use super::super::subs::handle_interject_hold;
use super::super::{control_loop, test_state, Control, Reader, ServerState, Writer};
use crate::agents::antigravity_hook::{wire, Ask, HookEvent};
use crate::agents::AgentKind;
use crate::ipc::{self, ClientMsg, InterjectAction, ServerMsg};

const ROOT: &str = "11111111-1111-4111-8111-111111111111";
const CHILD: &str = "22222222-2222-4222-8222-222222222222";
const OTHER: &str = "44444444-4444-4444-8444-444444444444";
const PARENT_TRANSCRIPT: &str = include_str!("fixtures/parent-transcript.jsonl");

/// A daemon, and the home directory it finds the transcripts under.
struct World {
    state: Arc<ServerState>,
    home: tempfile::TempDir,
    _directory: tempfile::TempDir,
}

fn world() -> World {
    let directory = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let state = test_state(directory.path());
    state.antigravity_trees.look_in(home.path());
    World {
        state,
        home,
        _directory: directory,
    }
}

fn transcript(home: &Path, id: &str) -> PathBuf {
    home.join(".gemini/antigravity/brain")
        .join(id)
        .join(".system_generated/logs/transcript_full.jsonl")
}

fn payload(home: &Path, id: &str, hook: &str, extra: Value) -> Value {
    let mut payload = json!({
        "hookEventName": hook,
        "conversationId": id,
        "transcriptPath": transcript(home, id),
        "workspacePaths": ["/work/project"],
    });
    for (key, value) in extra.as_object().unwrap() {
        payload[key] = value.clone();
    }
    payload
}

impl World {
    fn start(&self, id: &str) -> ClientMsg {
        let input = payload(self.home.path(), id, "SessionStart", json!({}));
        wire(HookEvent::SessionStart, &input, Ask::Nothing)
    }

    fn invocation(&self, id: &str, number: u64, ask: Ask) -> ClientMsg {
        let extra = json!({"invocationNum": number, "lastUserInput": "go"});
        let input = payload(self.home.path(), id, "PreInvocation", extra);
        wire(HookEvent::PreInvocation, &input, ask)
    }

    fn stop(&self, id: &str, idle: bool, ask: Ask) -> ClientMsg {
        let extra = json!({"fullyIdle": idle, "terminationReason": "NO_TOOL_CALL"});
        let input = payload(self.home.path(), id, "Stop", extra);
        wire(HookEvent::Stop, &input, ask)
    }

    /// The sessions the registry shows, with their states.
    fn sessions(&self) -> Vec<(String, String)> {
        let mut rows: Vec<_> = self
            .state
            .agents
            .snapshot()
            .as_array()
            .unwrap()
            .iter()
            .map(|record| {
                (
                    record["sessionId"].as_str().unwrap().to_string(),
                    record["state"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    fn session(&self, id: &str, state: &str) -> (String, String) {
        (id.to_string(), state.to_string())
    }
}

/// The hook's end of a connection to the daemon.
struct Hook {
    reader: ipc::MessageReader<BufReader<DuplexStream>>,
    writer: DuplexStream,
}

impl Hook {
    async fn frame(&mut self) -> (InterjectAction, String) {
        match tokio::time::timeout(Duration::from_secs(5), self.reader.read::<ServerMsg>()).await {
            Ok(Ok(Some(ServerMsg::InterjectDecision { action, text, .. }))) => (action, text),
            _ => panic!("the daemon did not send a decision"),
        }
    }
}

fn connection() -> (Reader, Writer, Hook) {
    let (hook_out, daemon_in) = tokio::io::duplex(1 << 20);
    let (daemon_out, hook_in) = tokio::io::duplex(1 << 20);
    (
        Box::new(BufReader::new(daemon_in)),
        Box::new(daemon_out),
        Hook {
            reader: ipc::MessageReader::new(BufReader::new(hook_in)),
            writer: hook_out,
        },
    )
}

/// Plays one hook invocation the daemon answers at once, or not at all: the decision frames it
/// sent.
async fn call(state: &Arc<ServerState>, msg: ClientMsg) -> Vec<(InterjectAction, String)> {
    let (mut reader, mut writer, mut hook) = connection();
    ipc::write_msg(&mut hook.writer, &msg).await.unwrap();
    hook.writer.shutdown().await.unwrap();
    let control = control_loop(&mut reader, &mut writer, state).await;
    assert!(
        matches!(control, Control::Closed),
        "the daemon kept the connection"
    );
    drop(writer);
    let mut frames = Vec::new();
    while let Ok(Some(msg)) = hook.reader.read::<ServerMsg>().await {
        if let ServerMsg::InterjectDecision { action, text, .. } = msg {
            frames.push((action, text));
        }
    }
    frames
}

/// A hook the daemon holds: it has been told to wait and the connection stays open.
struct Held {
    hook: Hook,
    task: tokio::task::JoinHandle<()>,
}

async fn call_held(state: &Arc<ServerState>, msg: ClientMsg) -> Held {
    let (mut reader, mut writer, mut hook) = connection();
    ipc::write_msg(&mut hook.writer, &msg).await.unwrap();
    let Control::InterjectHold { session_id, rx } =
        control_loop(&mut reader, &mut writer, state).await
    else {
        panic!("the daemon did not hold the hook");
    };
    assert_eq!(hook.frame().await.0, InterjectAction::Hold);
    // `handle_conn` has counted the connection; the hold hands the count back while it waits.
    state
        .active
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let state = state.clone();
    let task =
        tokio::spawn(
            async move { handle_interject_hold(reader, writer, &state, session_id, rx).await },
        );
    Held { hook, task }
}

#[tokio::test]
async fn a_sub_agent_found_in_the_log_is_folded_into_its_root() {
    let world = world();
    let log = transcript(world.home.path(), ROOT);
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, PARENT_TRANSCRIPT).unwrap();

    // The sub-agent's own first hook can come before anything says whose it is.
    call(&world.state, world.start(CHILD)).await;
    assert_eq!(world.sessions(), [world.session(CHILD, "idle")]);
    // The root's next hook finds the creation in its log: the child stops being a session.
    call(&world.state, world.invocation(ROOT, 0, Ask::Nothing)).await;
    assert_eq!(world.sessions(), [world.session(ROOT, "working")]);
    // And what it does from now on is not a session either.
    call(&world.state, world.invocation(CHILD, 0, Ask::Nothing)).await;
    call(&world.state, world.invocation(CHILD, 1, Ask::Nothing)).await;
    assert_eq!(world.sessions(), [world.session(ROOT, "working")]);
}

#[tokio::test]
async fn the_root_ends_its_turn_with_its_last_sub_agent() {
    let world = world();
    world.state.antigravity_trees.link(ROOT, CHILD);
    call(&world.state, world.start(ROOT)).await;
    call(&world.state, world.invocation(ROOT, 0, Ask::Nothing)).await;
    call(&world.state, world.start(CHILD)).await;
    call(&world.state, world.invocation(CHILD, 0, Ask::Nothing)).await;
    // The root stopped with the sub-agent still working: that ends nothing.
    call(&world.state, world.stop(ROOT, false, Ask::Nothing)).await;
    assert_eq!(world.sessions(), [world.session(ROOT, "working")]);
    // The sub-agent's own end is the root's.
    call(&world.state, world.stop(CHILD, true, Ask::Nothing)).await;
    assert_eq!(world.sessions(), [world.session(ROOT, "idle")]);
    let result = world
        .state
        .agents
        .last_turn_result(AgentKind::Antigravity, ROOT)
        .expect("the root has a result");
    assert_eq!(
        result.outcome,
        crate::agents::result::TurnOutcome::Completed
    );
}

#[tokio::test]
async fn a_root_that_wakes_up_again_is_not_ended_by_its_sub_agent() {
    let world = world();
    world.state.antigravity_trees.link(ROOT, CHILD);
    call(&world.state, world.start(ROOT)).await;
    call(&world.state, world.invocation(ROOT, 0, Ask::Nothing)).await;
    call(&world.state, world.stop(ROOT, false, Ask::Nothing)).await;
    // The root runs again (a message from the sub-agent woke it) before the sub-agent ends.
    call(&world.state, world.invocation(ROOT, 0, Ask::Nothing)).await;
    call(&world.state, world.stop(CHILD, true, Ask::Nothing)).await;
    assert_eq!(world.sessions(), [world.session(ROOT, "working")]);
}

#[tokio::test]
async fn a_stop_waits_at_the_gate_until_the_last_sub_agent_has_ended() {
    let world = world();
    world.state.antigravity_trees.link(ROOT, CHILD);
    call(&world.state, world.start(ROOT)).await;
    call(&world.state, world.invocation(ROOT, 0, Ask::Nothing)).await;
    call(&world.state, world.invocation(CHILD, 0, Ask::Nothing)).await;

    // The root stopped with the sub-agent still working: the hook is told to wait.
    let mut held = call_held(&world.state, world.stop(ROOT, false, Ask::StopGate)).await;
    // The sub-agent ends: the waiting hook is told to go ahead and ask.
    call(&world.state, world.stop(CHILD, true, Ask::Nothing)).await;
    assert_eq!(held.hook.frame().await.0, InterjectAction::Release);
    held.task.await.unwrap();
    // The hook reports the end itself once the human has decided; until then the root works.
    assert_eq!(world.sessions(), [world.session(ROOT, "working")]);
    call(&world.state, world.stop(ROOT, true, Ask::Nothing)).await;
    assert_eq!(world.sessions(), [world.session(ROOT, "idle")]);
}

#[tokio::test]
async fn the_gate_skips_a_sub_agent_and_lets_a_conversation_without_any_ask() {
    let world = world();
    world.state.antigravity_trees.link(ROOT, CHILD);
    let skipped = call(&world.state, world.stop(CHILD, true, Ask::StopGate)).await;
    assert_eq!(skipped, [(InterjectAction::Skip, String::new())]);
    let asked = call(&world.state, world.stop(OTHER, true, Ask::StopGate)).await;
    assert_eq!(asked, [(InterjectAction::None, String::new())]);
    // The gate answers; it never applies what it was asked about.
    assert!(world.sessions().is_empty());
}

#[tokio::test]
async fn a_queued_message_is_handed_to_the_invocation_that_starts_the_turn() {
    let world = world();
    world
        .state
        .interject
        .submit(ROOT, "also check the logs", Vec::new());
    call(&world.state, world.start(ROOT)).await;
    let frames = call(&world.state, world.invocation(ROOT, 0, Ask::Interjection)).await;
    assert_eq!(
        frames,
        [(InterjectAction::Message, "also check the logs".to_string())]
    );
    // The invocation started the turn as well, and nothing is left to deliver.
    assert_eq!(world.sessions(), [world.session(ROOT, "working")]);
    assert_eq!(world.state.interject.pending_count(ROOT), 0);
    let again = call(&world.state, world.invocation(ROOT, 1, Ask::Interjection)).await;
    assert_eq!(again, [(InterjectAction::None, String::new())]);
}

#[tokio::test]
async fn an_open_composer_holds_the_invocation_without_losing_the_turn_start() {
    let world = world();
    world.state.interject.composer_opened(ROOT);
    call(&world.state, world.start(ROOT)).await;
    let mut held = call_held(&world.state, world.invocation(ROOT, 0, Ask::Interjection)).await;
    // The event was applied before the hook was held.
    assert_eq!(world.sessions(), [world.session(ROOT, "working")]);
    world.state.interject.composer_closed(ROOT);
    assert_eq!(held.hook.frame().await.0, InterjectAction::Release);
    held.task.await.unwrap();
}
