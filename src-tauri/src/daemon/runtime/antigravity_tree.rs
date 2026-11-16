//! Sub-agent conversations of Antigravity, folded into the conversation that started them
//! (`docs/specs/antigravity-deep-integration.md`, D17).
//!
//! A sub-agent is a conversation of its own with its own hooks. Left alone it would show up as a
//! second session, raise its own confirmation card, and "end its turn" while the conversation the
//! person is watching goes on. The host never says which conversation started which, but the
//! transcripts do: the result of the parent's `invoke_subagent` call names the new conversation,
//! and the same files say which messages were sent between conversations and which were read.
//!
//! A *tree* is a root (the conversation the person talks to) and its descendants. Descendants are
//! never sessions of their own: the registry ignores their events (`Observation::subagent`) and
//! their activity only keeps the root alive. A root whose last `Stop` left work running
//! (`fullyIdle == false`) has not ended its turn; it ends when
//!
//! * every descendant has reported its own final `Stop`, and
//! * no message between two conversations of the tree is still unread (the root is woken by it).
//!
//! A `Stop` hook that wants to confirm the end is held at a gate until then, and told to ask, or
//! not to when the root will wake up again anyway. Everything here is in memory: after a restart a
//! tree is found again from the transcripts at its next event.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::oneshot;

use super::{broadcast_agents_state, ServerState};
use crate::agents::interject::WaitOutcome;
use crate::agents::result::{Observation, TurnResult};
use crate::agents::{AgentKind, LifecycleEvent};

/// A conversation that reported nothing for this long no longer counts as running: the host died
/// or the conversation was dropped, and nothing will say so. The registry gives up on a silent
/// working session after the same time.
const STALE: Duration = Duration::from_secs(crate::agents::registry::WORKING_BACKSTOP_SECS);
/// A message nobody read for this long stops counting as unread. A root reads one within seconds
/// of going idle; a longer silence means the host dropped it, or this module misses how it was
/// read, and neither may keep a turn from ever ending.
const UNREAD_GRACE: Duration = Duration::from_secs(2 * 60);
/// How long a tree that reported nothing is remembered.
const FORGET_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// One pass over a transcript reads at most this much of what was appended since the last one.
const MAX_SCAN_BYTES: u64 = 8 * 1024 * 1024;
/// A tree is a few levels deep at most; this only guards the walk to the root against a cycle.
const MAX_DEPTH: usize = 16;

/// What a hook event says about the conversation it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Seen {
    /// It is working (a session start, an invocation).
    Running,
    /// Its `Stop` left work running; this is how its turn ends once that work is done.
    Waiting(TurnResult),
    /// Its `Stop` ended everything it was doing.
    Final,
}

/// What the tree makes of one event.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Recorded {
    /// The conversation is a sub-agent of another one.
    pub child: bool,
    /// The root whose tree the event may have finished.
    pub root: Option<String>,
}

/// The answer to a `Stop` hook that wants to confirm the end.
pub(super) enum Gate {
    /// Ask the human now.
    Card,
    /// Do not ask: a sub-agent's own end, a pause for background work, or a root that will be woken
    /// again by a message it has not read.
    Skip,
    /// A sub-agent is still running: wait for the answer.
    Hold(oneshot::Receiver<WaitOutcome>),
}

struct Node {
    /// When the conversation last reported anything.
    seen: Instant,
    /// It reported something and has not reported a final `Stop` since. Only a descendant's flag
    /// matters; a new descendant is running until it says otherwise.
    running: bool,
    /// A root's last `Stop` left work running: how its turn ends once that work is done.
    end: Option<TurnResult>,
    /// `Stop` hooks waiting at the gate for the tree to go idle.
    holds: Vec<oneshot::Sender<WaitOutcome>>,
}

impl Node {
    fn new(running: bool) -> Self {
        Node {
            seen: Instant::now(),
            running,
            end: None,
            holds: Vec::new(),
        }
    }

    /// Ends every waiting hook with `outcome`; returns how many were still listening.
    fn release(&mut self, outcome: impl Fn() -> WaitOutcome) -> usize {
        self.holds
            .drain(..)
            .map(|hold| hold.send(outcome()).is_ok())
            .filter(|listening| *listening)
            .count()
    }
}

struct Sent {
    count: u32,
    at: Instant,
}

#[derive(Default)]
struct Inner {
    /// Sub-agent → the conversation that created it.
    parent: HashMap<String, String>,
    /// Every conversation of a tree.
    nodes: HashMap<String, Node>,
    /// How far each transcript has been read.
    offsets: HashMap<String, u64>,
    /// (sender, recipient) → messages sent.
    sent: HashMap<(String, String), Sent>,
    /// (reader, sender) → messages read.
    read: HashMap<(String, String), u32>,
}

impl Inner {
    fn root_of(&self, id: &str) -> String {
        let mut at = id;
        for _ in 0..MAX_DEPTH {
            match self.parent.get(at) {
                Some(parent) => at = parent,
                None => break,
            }
        }
        at.to_string()
    }

    /// Every conversation below `of`, at any depth.
    fn descendants(&self, of: &str) -> Vec<String> {
        self.parent
            .keys()
            .filter(|child| self.is_below(child, of))
            .cloned()
            .collect()
    }

    fn is_below(&self, id: &str, of: &str) -> bool {
        let mut at = id;
        for _ in 0..MAX_DEPTH {
            match self.parent.get(at) {
                Some(above) if above == of => return true,
                Some(above) => at = above,
                None => return false,
            }
        }
        false
    }

    fn running_descendants(&self, root: &str) -> bool {
        let now = Instant::now();
        self.descendants(root).iter().any(|child| {
            self.nodes
                .get(child)
                .is_some_and(|node| node.running && now.duration_since(node.seen) < STALE)
        })
    }

    /// Whether any message between two conversations of the tree is still unread.
    fn unread(&self, root: &str) -> bool {
        let mut members = self.descendants(root);
        members.push(root.to_string());
        let now = Instant::now();
        self.sent.iter().any(|((from, to), sent)| {
            members.contains(from)
                && members.contains(to)
                && now.duration_since(sent.at) < UNREAD_GRACE
                && sent.count
                    > self
                        .read
                        .get(&(to.clone(), from.clone()))
                        .copied()
                        .unwrap_or(0)
        })
    }

    /// The end of `root`'s turn, if everything it waited for is done. Hooks held at the gate are
    /// answered first: they ask the human (or are told not to), and the registry's turn then ends
    /// the way their decision says.
    fn settle(&mut self, root: &str) -> Option<TurnResult> {
        let running = self.running_descendants(root);
        let unread = self.unread(root);
        let node = self.nodes.get_mut(root)?;
        if !node.holds.is_empty() {
            if running {
                return None;
            }
            let outcome = || {
                if unread {
                    WaitOutcome::Skip
                } else {
                    WaitOutcome::Release
                }
            };
            if node.release(outcome) > 0 {
                node.end = None;
                return None;
            }
        }
        if running || unread {
            return None;
        }
        node.end.take()
    }
}

/// The groups of conversations the daemon knows of.
#[derive(Default)]
pub(super) struct Trees {
    inner: Mutex<Inner>,
    /// Where tests keep the transcripts, instead of the account's Antigravity directories.
    #[cfg(test)]
    home: Mutex<Option<std::path::PathBuf>>,
}

impl Trees {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The step log of a conversation, wherever the host keeps it.
    fn transcript(&self, id: &str) -> Option<std::path::PathBuf> {
        #[cfg(test)]
        if let Some(home) = self.home.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return crate::agents::title::antigravity_transcript_in(&home, id);
        }
        crate::agents::title::transcript_path(AgentKind::Antigravity, id)
    }

    #[cfg(test)]
    pub fn look_in(&self, home: &Path) {
        *self.home.lock().unwrap_or_else(|e| e.into_inner()) = Some(home.to_path_buf());
    }

    /// Records that `parent` created `child`. Returns whether the link is new. A conversation has
    /// one parent (the first one that named it), and never an ancestor of its own as a child.
    pub fn link(&self, parent: &str, child: &str) -> bool {
        if parent == child {
            return false;
        }
        let mut inner = self.lock();
        if inner.parent.contains_key(child) {
            return false;
        }
        let mut at = parent;
        for _ in 0..MAX_DEPTH {
            if at == child {
                return false;
            }
            match inner.parent.get(at) {
                Some(above) => at = above,
                None => break,
            }
        }
        inner.parent.insert(child.to_string(), parent.to_string());
        inner
            .nodes
            .entry(parent.to_string())
            .or_insert_with(|| Node::new(false));
        // A conversation that was just created is about to run, even before its first hook.
        inner.nodes.insert(child.to_string(), Node::new(true));
        true
    }

    /// The conversation and every one below it, for reading their transcripts.
    pub fn tree_of(&self, id: &str) -> Vec<String> {
        let inner = self.lock();
        let mut ids = vec![id.to_string()];
        ids.extend(inner.descendants(id));
        ids
    }

    fn offset(&self, id: &str) -> u64 {
        self.lock().offsets.get(id).copied().unwrap_or(0)
    }

    /// Takes in what a pass over `id`'s transcript found; returns the conversations it linked.
    /// `restarted` means the file was shorter than the last offset (replaced): what this
    /// conversation had sent or read is counted again from the start.
    fn apply(&self, id: &str, found: Found, offset: u64, restarted: bool) -> Vec<String> {
        let mut inner = self.lock();
        inner.offsets.insert(id.to_string(), offset);
        if restarted {
            inner.sent.retain(|(from, _), _| from != id);
            inner.read.retain(|(reader, _), _| reader != id);
        }
        let now = Instant::now();
        for recipient in found.sent {
            let sent = inner
                .sent
                .entry((id.to_string(), recipient))
                .or_insert(Sent { count: 0, at: now });
            sent.count += 1;
            sent.at = now;
        }
        for sender in found.read {
            *inner.read.entry((id.to_string(), sender)).or_insert(0) += 1;
        }
        drop(inner);
        found
            .created
            .into_iter()
            .filter(|child| self.link(id, child))
            .collect()
    }

    #[cfg(test)]
    pub fn is_child(&self, id: &str) -> bool {
        self.lock().parent.contains_key(id)
    }

    /// Whether the conversation is part of a tree, as a root or as a sub-agent.
    fn in_tree(&self, id: &str) -> bool {
        self.lock().nodes.contains_key(id)
    }

    /// Takes in one event. A conversation outside every tree is left alone entirely.
    pub fn record(&self, id: &str, seen: Seen) -> Recorded {
        let mut inner = self.lock();
        let child = inner.parent.contains_key(id);
        if !child && !inner.nodes.contains_key(id) {
            return Recorded::default();
        }
        let root = inner.root_of(id);
        let Some(node) = inner.nodes.get_mut(id) else {
            return Recorded::default();
        };
        node.seen = Instant::now();
        if child {
            node.running = seen != Seen::Final;
        } else {
            match seen {
                Seen::Waiting(end) => node.end = Some(end),
                // A root that works again has nothing left to wait for, and a hook still held at the
                // gate is for a Stop that no longer is the last word.
                Seen::Running | Seen::Final => {
                    node.end = None;
                    node.release(|| WaitOutcome::Skip);
                }
            }
        }
        Recorded {
            child,
            root: Some(root),
        }
    }

    /// The end of `root`'s turn, once everything it waited for is done.
    pub fn settle(&self, root: &str) -> Option<TurnResult> {
        self.lock().settle(root)
    }

    /// What a `Stop` hook that wants to confirm the end is told. `end` is how the turn ends when
    /// the `Stop` left work running (`final_stop` is false).
    pub fn gate(&self, id: &str, final_stop: bool, end: Option<TurnResult>) -> Gate {
        let mut inner = self.lock();
        if inner.parent.contains_key(id) {
            return Gate::Skip;
        }
        if !inner.nodes.contains_key(id) {
            // No sub-agent: a final Stop is the end, and a Stop that left work running is a pause
            // for background work that is not the human's to confirm.
            return if final_stop { Gate::Card } else { Gate::Skip };
        }
        if inner.unread(id) {
            return Gate::Skip;
        }
        if final_stop || !inner.running_descendants(id) {
            return Gate::Card;
        }
        let (hold, wait) = oneshot::channel();
        let Some(node) = inner.nodes.get_mut(id) else {
            return Gate::Skip;
        };
        node.end = end;
        node.holds.push(hold);
        Gate::Hold(wait)
    }

    /// Settles what time alone has decided (a sub-agent that went silent), and forgets trees nobody
    /// has heard of for a day. Returns the roots whose turn ended.
    pub fn sweep(&self) -> Vec<(String, TurnResult)> {
        let mut inner = self.lock();
        let roots: Vec<String> = inner
            .nodes
            .iter()
            .filter(|(id, node)| {
                !inner.parent.contains_key(*id) && (node.end.is_some() || !node.holds.is_empty())
            })
            .map(|(id, _)| id.clone())
            .collect();
        let ended = roots
            .into_iter()
            .filter_map(|root| inner.settle(&root).map(|end| (root, end)))
            .collect();
        let now = Instant::now();
        let stale: Vec<String> = inner
            .nodes
            .iter()
            .filter(|(id, node)| {
                // A tree is forgotten as a whole, so look at the newest of its conversations.
                let root = inner.root_of(id);
                let newest = std::iter::once(root.clone())
                    .chain(inner.descendants(&root))
                    .filter_map(|member| inner.nodes.get(&member).map(|node| node.seen))
                    .max()
                    .unwrap_or(node.seen);
                now.duration_since(newest) > FORGET_AFTER
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            inner.nodes.remove(id);
            inner.parent.remove(id);
            inner.offsets.remove(id);
            inner.sent.retain(|(from, to), _| from != id && to != id);
            inner
                .read
                .retain(|(reader, sender), _| reader != id && sender != id);
        }
        ended
    }
}

/// What one pass over a transcript found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Found {
    /// Conversations this one created.
    pub created: Vec<String>,
    /// Recipients of the messages it sent.
    pub sent: Vec<String>,
    /// Senders of the messages it read.
    pub read: Vec<String>,
}

/// The result of an `invoke_subagent` call begins with this line (after the step's timestamps).
const CREATED: &str = "Created the following subagent:";
/// A message reaches a conversation as a system message with one of these headers per message.
const MESSAGE: &str = "[Message] ";

fn normalized(id: &str) -> Option<String> {
    uuid::Uuid::parse_str(id.trim())
        .ok()
        .map(|id| id.hyphenated().to_string())
}

/// The conversation a finished `invoke_subagent` step created. Only the first line of the result
/// counts, so a command that merely prints a transcript containing the sentence does not.
fn created_by(step: &Value) -> Option<String> {
    let content = step.get("content")?.as_str()?;
    let body = content
        .lines()
        .skip_while(|line| line.starts_with("Created At: ") || line.starts_with("Completed At: "))
        .collect::<Vec<_>>()
        .join("\n");
    let rest = body.strip_prefix(CREATED)?;
    let object = rest.find('{').map(|start| &rest[start..])?;
    let created: Value = serde_json::Deserializer::from_str(object)
        .into_iter::<Value>()
        .next()?
        .ok()?;
    normalized(created.get("conversationId")?.as_str()?)
}

/// The senders of the messages a system message delivers: one header per message, each at the
/// start of a line.
fn senders_of(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| line.strip_prefix(MESSAGE))
        // The header (timestamp, sender, priority) comes before `content=`, which may say anything.
        .filter_map(|header| {
            header
                .split(" content=")
                .next()?
                .split_whitespace()
                .find_map(|part| part.strip_prefix("sender="))
                .and_then(normalized)
        })
        .collect()
}

fn scan_line(line: &str, found: &mut Found) {
    if !(line.contains(CREATED) || line.contains("send_message") || line.contains(MESSAGE)) {
        return;
    }
    let Ok(step) = serde_json::from_str::<Value>(line) else {
        return;
    };
    // A step that is still being written is counted when it is done.
    if step.get("status").and_then(Value::as_str) != Some("DONE") {
        return;
    }
    match step.get("type").and_then(Value::as_str) {
        Some("GENERIC") => found.created.extend(created_by(&step)),
        Some("PLANNER_RESPONSE") => {
            let calls = step.get("tool_calls").and_then(Value::as_array);
            for call in calls.into_iter().flatten() {
                if call.get("name").and_then(Value::as_str) == Some("send_message") {
                    let recipient = call["args"]["Recipient"].as_str().and_then(normalized);
                    found.sent.extend(recipient);
                }
            }
        }
        Some("SYSTEM_MESSAGE") => {
            if let Some(content) = step.get("content").and_then(Value::as_str) {
                found.read.extend(senders_of(content));
            }
        }
        _ => {}
    }
}

/// Reads the complete lines appended to a transcript since `from`. Returns what they say, the new
/// offset, and whether the file was shorter than `from` (so it was read from its start again).
fn scan_file(path: &Path, from: u64) -> (Found, u64, bool) {
    let mut found = Found::default();
    let Ok(mut file) = std::fs::File::open(path) else {
        return (found, from, false);
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return (found, from, false);
    };
    let restarted = from > len;
    let start = if restarted { 0 } else { from };
    if start == len || file.seek(SeekFrom::Start(start)).is_err() {
        return (found, start, restarted);
    }
    let mut buffer = Vec::new();
    if file.take(MAX_SCAN_BYTES).read_to_end(&mut buffer).is_err() {
        return (found, start, restarted);
    }
    // Only a complete line is read: the host may be in the middle of writing the last one. A chunk
    // without any line break that fills the whole allowance is one enormous step (a tool's output);
    // it is skipped rather than blocking every later step.
    let consumed = match buffer.iter().rposition(|byte| *byte == b'\n') {
        Some(at) => at + 1,
        None if buffer.len() as u64 >= MAX_SCAN_BYTES => buffer.len(),
        None => 0,
    };
    for line in String::from_utf8_lossy(&buffer[..consumed]).lines() {
        scan_line(line, &mut found);
    }
    (found, start + consumed as u64, restarted)
}

/// Reads what the transcripts of `ids` gained since the last look. A conversation found to be a
/// sub-agent stops being a session of its own: its record, which a hook of its own may already
/// have created, is dropped.
async fn refresh(state: &Arc<ServerState>, ids: Vec<String>) {
    for id in ids {
        let Some(path) = state.antigravity_trees.transcript(&id) else {
            continue;
        };
        let from = state.antigravity_trees.offset(&id);
        let Ok((found, offset, restarted)) =
            tokio::task::spawn_blocking(move || scan_file(&path, from)).await
        else {
            continue;
        };
        for child in state.antigravity_trees.apply(&id, found, offset, restarted) {
            if state.agents.forget_session(AgentKind::Antigravity, &child) {
                state.agents.persist();
                broadcast_agents_state(state);
            }
        }
    }
}

/// What one hook event told the tree, kept until its follow-up.
pub(super) struct Folded {
    /// The conversation is a sub-agent: the registry must ignore the event.
    pub child: bool,
    seen: Seen,
    root: Option<String>,
    /// The event may have named a sub-agent (a session start has no transcript yet).
    scan: bool,
}

/// Takes in one event before the registry sees it. Cheap and synchronous: a hook is usually
/// waiting for the daemon's answer. A sub-agent's event keeps its root alive.
pub(super) fn record(
    state: &Arc<ServerState>,
    id: &str,
    event: LifecycleEvent,
    observation: Option<&Observation>,
) -> Option<Folded> {
    let seen = match event {
        LifecycleEvent::TurnEnd => Seen::Final,
        LifecycleEvent::Activity => match observation.and_then(|o| o.deferred_end.clone()) {
            Some(end) => Seen::Waiting(end),
            None => Seen::Running,
        },
        LifecycleEvent::SessionStart | LifecycleEvent::TurnStart => Seen::Running,
        _ => return None,
    };
    let recorded = state.antigravity_trees.record(id, seen.clone());
    if let (true, Some(root)) = (recorded.child, recorded.root.as_deref()) {
        state
            .agents
            .touch_activity(AgentKind::Antigravity, root, None);
    }
    Some(Folded {
        child: recorded.child,
        seen,
        root: recorded.root,
        scan: event != LifecycleEvent::SessionStart,
    })
}

/// After an event, once its hook has been answered: finds the sub-agents the conversation created
/// by now, and ends the root's turn if everything it waited for is done.
pub(super) async fn follow_up(state: &Arc<ServerState>, id: &str, folded: Folded) {
    let mut root = folded.root;
    if folded.scan {
        refresh(state, vec![id.to_string()]).await;
        // The event itself may be what made the conversation a root: a `Stop` can be the first
        // thing to show its sub-agents.
        if root.is_none() && state.antigravity_trees.in_tree(id) {
            root = state.antigravity_trees.record(id, folded.seen).root;
        }
    }
    let Some(root) = root else {
        return;
    };
    refresh(state, state.antigravity_trees.tree_of(&root)).await;
    if let Some(end) = state.antigravity_trees.settle(&root) {
        end_turn(state, &root, end);
    }
}

/// Answers a `Stop` hook that wants to confirm the end of `id`'s turn.
pub(super) async fn gate(
    state: &Arc<ServerState>,
    id: &str,
    event: LifecycleEvent,
    observation: Option<&Observation>,
) -> Gate {
    // What the conversation created so far, then what the whole tree sent and read.
    refresh(state, vec![id.to_string()]).await;
    refresh(state, state.antigravity_trees.tree_of(id)).await;
    state.antigravity_trees.gate(
        id,
        event == LifecycleEvent::TurnEnd,
        observation.and_then(|o| o.deferred_end.clone()),
    )
}

/// Ends what time alone has decided; called with the registry's own periodic sweeps.
pub(super) fn sweep(state: &Arc<ServerState>) {
    for (root, end) in state.antigravity_trees.sweep() {
        end_turn(state, &root, end);
    }
}

fn end_turn(state: &Arc<ServerState>, root: &str, end: TurnResult) {
    let observation = Observation {
        result: Some(end),
        ..Observation::default()
    };
    super::agent_control::apply(
        state,
        AgentKind::Antigravity,
        root,
        LifecycleEvent::TurnEnd,
        observation,
        None,
    );
    broadcast_agents_state(state);
    state.watch.notify.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::result::TurnOutcome;

    const ROOT: &str = "11111111-1111-4111-8111-111111111111";
    const CHILD: &str = "22222222-2222-4222-8222-222222222222";
    const GRANDCHILD: &str = "33333333-3333-4333-8333-333333333333";
    const PARENT_TRANSCRIPT: &str =
        include_str!("antigravity_tree/fixtures/parent-transcript.jsonl");

    fn end(outcome: TurnOutcome) -> TurnResult {
        TurnResult {
            id: String::new(),
            outcome,
            source: Some("hook".into()),
            turn_id: None,
            reason: None,
            summary: None,
            at: 0,
        }
    }

    fn found(created: &[&str], sent: &[&str], read: &[&str]) -> Found {
        let owned = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect();
        Found {
            created: owned(created),
            sent: owned(sent),
            read: owned(read),
        }
    }

    // ---- reading transcripts ----

    #[test]
    fn the_result_of_invoke_subagent_names_the_conversation_it_created() {
        let mut found = Found::default();
        for line in PARENT_TRANSCRIPT.lines() {
            scan_line(line, &mut found);
        }
        // The recorded run (ids sanitized): one creation, one message the root read.
        assert_eq!(found.created, [CHILD]);
        assert_eq!(found.read, [CHILD]);
        assert!(found.sent.is_empty());
    }

    #[test]
    fn a_command_that_prints_the_sentence_does_not_create_a_conversation() {
        // The sentence in the middle of a command's output is not the result of the tool.
        let output = format!(
            "Created At: 2026-11-16T02:07:21+08:00\nCompleted At: 2026-11-16T02:07:21+08:00\nCommand output:\n{CREATED}\n{{\"conversationId\": \"{GRANDCHILD}\"}}"
        );
        let step = serde_json::json!({
            "type": "GENERIC", "source": "MODEL", "status": "DONE", "content": output,
        });
        let mut found = Found::default();
        scan_line(&step.to_string(), &mut found);
        assert!(found.created.is_empty());
        // Nor does one that is still running, or an id that is not a conversation id.
        let running = serde_json::json!({
            "type": "GENERIC", "status": "RUNNING",
            "content": format!("{CREATED}\n{{\"conversationId\": \"{GRANDCHILD}\"}}"),
        });
        scan_line(&running.to_string(), &mut found);
        let bogus = serde_json::json!({
            "type": "GENERIC", "status": "DONE",
            "content": format!("{CREATED}\n{{\"conversationId\": \"../../etc\"}}"),
        });
        scan_line(&bogus.to_string(), &mut found);
        assert!(found.created.is_empty());
    }

    #[test]
    fn messages_are_counted_by_who_sent_them_and_who_read_them() {
        let sent = serde_json::json!({
            "type": "PLANNER_RESPONSE", "status": "DONE", "content": "",
            "tool_calls": [
                {"name": "send_message", "args": {"Message": "ok", "Recipient": ROOT}},
                {"name": "view_file", "args": {"AbsolutePath": "/x"}},
            ],
        });
        let two = serde_json::json!({
            "type": "SYSTEM_MESSAGE", "status": "DONE", "source": "SYSTEM",
            "content": format!(
                "The following is a <SYSTEM_MESSAGE> ...\n<SYSTEM_MESSAGE>\n[Message] timestamp=t sender={CHILD} priority=P content=sender={GRANDCHILD} [Message] decoy\n[Message] timestamp=t sender={GRANDCHILD} priority=P content=hi\n</SYSTEM_MESSAGE>"
            ),
        });
        let mut found = Found::default();
        scan_line(&sent.to_string(), &mut found);
        scan_line(&two.to_string(), &mut found);
        assert_eq!(found.sent, [ROOT]);
        // Each header counts once; whatever a message says in its own text is not a sender.
        assert_eq!(found.read, [CHILD, GRANDCHILD]);
    }

    #[test]
    fn an_unfinished_step_is_counted_once_it_is_done() {
        let unfinished = serde_json::json!({
            "type": "PLANNER_RESPONSE", "status": "GENERATING",
            "tool_calls": [{"name": "send_message", "args": {"Recipient": ROOT}}],
        });
        let mut found = Found::default();
        scan_line(&unfinished.to_string(), &mut found);
        assert!(found.sent.is_empty());
    }

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_pass_reads_only_complete_lines_and_resumes_where_it_stopped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transcript_full.jsonl");
        let lines: Vec<&str> = PARENT_TRANSCRIPT.lines().collect();
        // Everything but the last line, which the host is still writing.
        let (head, last) = lines.split_at(lines.len() - 1);
        let partial = &last[0][..last[0].len() / 2];
        write(&path, &format!("{}\n{partial}", head.join("\n")));
        let (first, offset, restarted) = scan_file(&path, 0);
        assert!(!restarted);
        assert_eq!(first.created, [CHILD]);
        assert!(first.read.is_empty(), "the last line was not complete yet");
        assert_eq!(offset as usize, head.join("\n").len() + 1);
        // The line is finished and one more arrives: only the new ones are read.
        write(&path, &format!("{}\n", lines.join("\n")));
        let (second, offset, _) = scan_file(&path, offset);
        assert!(second.created.is_empty());
        assert_eq!(second.read, [CHILD]);
        assert_eq!(offset as usize, PARENT_TRANSCRIPT.len());
        // Nothing new: nothing found and the offset stays.
        let (third, again, _) = scan_file(&path, offset);
        assert_eq!(third, Found::default());
        assert_eq!(again, offset);
    }

    #[test]
    fn a_shorter_file_is_read_again_from_the_start() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transcript_full.jsonl");
        write(&path, PARENT_TRANSCRIPT);
        let (_, offset, _) = scan_file(&path, 0);
        write(&path, "");
        let (found, restarted_at, restarted) = scan_file(&path, offset);
        assert!(restarted);
        assert_eq!((found, restarted_at), (Found::default(), 0));
        // A missing file is not news.
        let (found, same, restarted) = scan_file(&directory.path().join("missing"), 7);
        assert_eq!((found, same, restarted), (Found::default(), 7, false));
    }

    #[test]
    fn one_enormous_line_does_not_block_the_steps_after_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transcript_full.jsonl");
        let mut text = "x".repeat(MAX_SCAN_BYTES as usize);
        text.push('\n');
        text.push_str(PARENT_TRANSCRIPT);
        write(&path, &text);
        // The first pass fills its allowance with the line and skips it...
        let (first, offset, _) = scan_file(&path, 0);
        assert_eq!(first, Found::default());
        // ...and the next one reads what follows.
        let (second, _, _) = scan_file(&path, offset);
        assert_eq!(second.created, [CHILD]);
    }

    // ---- the trees ----

    #[test]
    fn a_conversation_has_one_parent_and_never_a_descendant_for_a_parent() {
        let trees = Trees::default();
        assert!(trees.link(ROOT, CHILD));
        assert!(!trees.link(ROOT, CHILD), "a known link is not new");
        assert!(!trees.link(GRANDCHILD, CHILD), "the first parent stays");
        assert!(trees.link(CHILD, GRANDCHILD));
        assert!(!trees.link(GRANDCHILD, ROOT), "no cycle");
        assert!(!trees.link(ROOT, ROOT));
        assert!(trees.is_child(CHILD) && trees.is_child(GRANDCHILD) && !trees.is_child(ROOT));
        let mut tree = trees.tree_of(ROOT);
        tree.sort();
        assert_eq!(tree, [ROOT, CHILD, GRANDCHILD]);
        assert_eq!(trees.tree_of(CHILD), [CHILD, GRANDCHILD]);
    }

    #[test]
    fn what_a_pass_found_links_and_counts() {
        let trees = Trees::default();
        let linked = trees.apply(ROOT, found(&[CHILD], &[], &[CHILD]), 100, false);
        assert_eq!(linked, [CHILD]);
        assert_eq!(trees.offset(ROOT), 100);
        // The same creation seen again is not new.
        assert!(trees
            .apply(ROOT, found(&[CHILD], &[], &[]), 200, false)
            .is_empty());
    }

    #[test]
    fn events_outside_every_tree_are_left_alone() {
        let trees = Trees::default();
        assert_eq!(trees.record(ROOT, Seen::Running), Recorded::default());
        assert_eq!(trees.record(ROOT, Seen::Final), Recorded::default());
        assert!(trees.settle(ROOT).is_none());
    }

    /// A root that stopped with work left, and its sub-agent.
    fn waiting_tree() -> Trees {
        let trees = Trees::default();
        trees.link(ROOT, CHILD);
        trees.record(ROOT, Seen::Waiting(end(TurnOutcome::Completed)));
        trees
    }

    #[test]
    fn the_root_ends_when_its_last_sub_agent_reports_its_final_stop() {
        let trees = waiting_tree();
        // The sub-agent has not said a word yet, and the root still waits.
        assert!(trees.settle(ROOT).is_none());
        let recorded = trees.record(CHILD, Seen::Running);
        assert_eq!(
            recorded,
            Recorded {
                child: true,
                root: Some(ROOT.into())
            }
        );
        assert!(trees.settle(ROOT).is_none());
        trees.record(CHILD, Seen::Waiting(end(TurnOutcome::Completed)));
        assert!(
            trees.settle(ROOT).is_none(),
            "a Stop that left work running is not final for a sub-agent either"
        );
        trees.record(CHILD, Seen::Final);
        let result = trees.settle(ROOT).expect("the whole tree is idle");
        assert_eq!(result.outcome, TurnOutcome::Completed);
        // The end is delivered once.
        assert!(trees.settle(ROOT).is_none());
    }

    #[test]
    fn the_end_keeps_the_outcome_of_the_stop_that_left_work_running() {
        let trees = Trees::default();
        trees.link(ROOT, CHILD);
        trees.record(ROOT, Seen::Waiting(end(TurnOutcome::Incomplete)));
        trees.record(CHILD, Seen::Final);
        assert_eq!(
            trees.settle(ROOT).map(|result| result.outcome),
            Some(TurnOutcome::Incomplete)
        );
    }

    #[test]
    fn a_root_that_works_again_has_nothing_left_to_wait_for() {
        let trees = waiting_tree();
        trees.record(ROOT, Seen::Running);
        trees.record(CHILD, Seen::Final);
        assert!(trees.settle(ROOT).is_none());
        // Its next Stop that leaves nothing running ends it for real, through its own hook.
        trees.record(ROOT, Seen::Final);
        assert!(trees.settle(ROOT).is_none());
    }

    #[test]
    fn every_level_of_the_tree_has_to_be_idle() {
        let trees = Trees::default();
        trees.link(ROOT, CHILD);
        trees.link(CHILD, GRANDCHILD);
        trees.record(ROOT, Seen::Waiting(end(TurnOutcome::Completed)));
        trees.record(CHILD, Seen::Final);
        assert!(
            trees.settle(ROOT).is_none(),
            "the grandchild is still running"
        );
        let recorded = trees.record(GRANDCHILD, Seen::Final);
        assert_eq!(recorded.root.as_deref(), Some(ROOT));
        assert!(trees.settle(ROOT).is_some());
    }

    #[test]
    fn a_message_nobody_read_yet_keeps_the_turn_open() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Final);
        // The sub-agent's last act was a message to the root, which is still blocked in its hook.
        trees.apply(CHILD, found(&[], &[ROOT], &[]), 10, false);
        assert!(trees.settle(ROOT).is_none(), "the root wakes for it");
        // The root reads it.
        trees.apply(ROOT, found(&[], &[], &[CHILD]), 10, false);
        assert!(trees.settle(ROOT).is_some());
    }

    #[test]
    fn a_message_nobody_reads_for_long_stops_counting() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Final);
        trees.apply(CHILD, found(&[], &[ROOT], &[]), 10, false);
        assert!(trees.settle(ROOT).is_none());
        // The host dropped it, or the log never showed it being read: it must not hold the turn
        // for ever.
        for sent in trees.lock().sent.values_mut() {
            sent.at = Instant::now() - UNREAD_GRACE - Duration::from_secs(1);
        }
        assert!(trees.settle(ROOT).is_some());
    }

    #[test]
    fn a_sub_agent_that_went_silent_stops_counting_as_running() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Running);
        assert!(trees.settle(ROOT).is_none());
        trees.lock().nodes.get_mut(CHILD).unwrap().seen =
            Instant::now() - STALE - Duration::from_secs(1);
        let ended = trees.sweep();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].0, ROOT);
    }

    #[test]
    fn a_tree_nobody_heard_of_for_a_day_is_forgotten_with_its_counts() {
        let trees = waiting_tree();
        trees.apply(CHILD, found(&[], &[ROOT], &[]), 10, false);
        {
            let mut inner = trees.lock();
            for node in inner.nodes.values_mut() {
                node.seen = Instant::now() - FORGET_AFTER - Duration::from_secs(1);
            }
        }
        trees.sweep();
        let inner = trees.lock();
        assert!(inner.nodes.is_empty() && inner.parent.is_empty());
        assert!(inner.offsets.is_empty() && inner.sent.is_empty() && inner.read.is_empty());
    }

    #[test]
    fn a_replaced_log_counts_its_messages_again_from_the_start() {
        let trees = waiting_tree();
        trees.apply(CHILD, found(&[], &[ROOT, ROOT], &[]), 10, false);
        trees.apply(ROOT, found(&[], &[], &[CHILD, CHILD]), 10, false);
        // The sender's file is replaced: its two messages are counted once more, so the root's two
        // readings no longer cover them... unless the replacement has them only once.
        trees.apply(CHILD, found(&[], &[ROOT], &[]), 5, true);
        assert_eq!(
            trees
                .lock()
                .sent
                .get(&(CHILD.into(), ROOT.into()))
                .map(|s| s.count),
            Some(1)
        );
    }

    // ---- the gate ----

    fn is_card(gate: &Gate) -> bool {
        matches!(gate, Gate::Card)
    }

    fn is_skip(gate: &Gate) -> bool {
        matches!(gate, Gate::Skip)
    }

    #[test]
    fn a_conversation_without_sub_agents_asks_only_when_it_is_really_done() {
        let trees = Trees::default();
        assert!(is_card(&trees.gate(ROOT, true, None)));
        // A Stop that left a background task running is not an end to confirm.
        assert!(is_skip(&trees.gate(
            ROOT,
            false,
            Some(end(TurnOutcome::Completed))
        )));
    }

    #[test]
    fn a_sub_agent_never_raises_a_card() {
        let trees = waiting_tree();
        assert!(is_skip(&trees.gate(CHILD, true, None)));
        assert!(is_skip(&trees.gate(CHILD, false, None)));
    }

    #[test]
    fn a_root_with_a_running_sub_agent_waits_and_is_then_told_to_ask() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Running);
        let Gate::Hold(mut wait) = trees.gate(ROOT, false, Some(end(TurnOutcome::Completed)))
        else {
            panic!("a sub-agent is still running");
        };
        assert!(wait.try_recv().is_err(), "nothing to say yet");
        trees.record(CHILD, Seen::Final);
        // The root did not settle: the hook that was waiting asks the human, and decides.
        assert!(trees.settle(ROOT).is_none());
        assert!(matches!(wait.try_recv(), Ok(WaitOutcome::Release)));
        // Nothing is left to settle behind the card's back.
        assert!(trees.settle(ROOT).is_none());
    }

    #[test]
    fn a_waiting_hook_is_told_not_to_ask_when_a_message_still_wakes_the_root() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Running);
        let Gate::Hold(mut wait) = trees.gate(ROOT, false, Some(end(TurnOutcome::Completed)))
        else {
            panic!("a sub-agent is still running");
        };
        // The sub-agent sent its result and finished while the root sat in the hook.
        trees.apply(CHILD, found(&[], &[ROOT], &[]), 10, false);
        trees.record(CHILD, Seen::Final);
        assert!(trees.settle(ROOT).is_none());
        assert!(matches!(wait.try_recv(), Ok(WaitOutcome::Skip)));
        // The root's own end is not settled yet: it wakes for the message first.
        assert!(trees.settle(ROOT).is_none());
    }

    #[test]
    fn a_waiting_hook_is_released_when_the_root_works_again() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Running);
        let Gate::Hold(mut wait) = trees.gate(ROOT, false, Some(end(TurnOutcome::Completed)))
        else {
            panic!("a sub-agent is still running");
        };
        trees.record(ROOT, Seen::Running);
        assert!(matches!(wait.try_recv(), Ok(WaitOutcome::Skip)));
    }

    #[test]
    fn a_hook_that_gave_up_waiting_does_not_hold_the_turn() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Running);
        let Gate::Hold(wait) = trees.gate(ROOT, false, Some(end(TurnOutcome::Completed))) else {
            panic!("a sub-agent is still running");
        };
        // The hook was killed: nobody listens any more.
        drop(wait);
        trees.record(CHILD, Seen::Final);
        assert!(trees.settle(ROOT).is_some(), "the turn ends without a card");
    }

    #[test]
    fn a_stop_that_ends_everything_still_waits_for_an_unread_message() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Final);
        trees.apply(CHILD, found(&[], &[ROOT], &[]), 10, false);
        assert!(is_skip(&trees.gate(ROOT, true, None)));
        trees.apply(ROOT, found(&[], &[], &[CHILD]), 10, false);
        assert!(is_card(&trees.gate(ROOT, true, None)));
    }

    #[test]
    fn an_idle_tree_asks_at_once() {
        let trees = waiting_tree();
        trees.record(CHILD, Seen::Final);
        assert!(is_card(&trees.gate(
            ROOT,
            false,
            Some(end(TurnOutcome::Completed))
        )));
    }
}

#[cfg(test)]
mod events;
