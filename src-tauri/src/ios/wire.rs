//! Wire protocol v1 constants, topics and message bodies (plan §2, §3.1, §3.4).
//!
//! Message bodies are JSON objects with camelCase keys. The Swift mirror lives in
//! `ios/Packages/AskHumanKit/Sources/AHProtocol`; `ios/Fixtures/wire-v1/messages.json` holds one
//! sample per message type that both implementations must decode and re-encode identically.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------- constants

pub const PROTO: u32 = 1;
/// Key id of the pairing-channel ticket key.
pub const KID_TICKET: u32 = 0;
/// Key id of the long-lived pair keys.
pub const KID_PAIR: u32 = 1;

/// EMQX caps a single message at 1 MB.
pub const MAX_ENVELOPE_BYTES: usize = 900_000;
pub const MAX_PLAIN_BYTES: usize = 512 * 1024;
/// Requests up to this size are sent in full; larger ones as summaries fetched with `request.get`.
pub const INLINE_REQUEST_BYTES: usize = 128 * 1024;
pub const THUMB_MAX_BYTES: usize = 24 * 1024;
pub const ASSET_CHUNK_BYTES: usize = 192 * 1024;
pub const MAX_ASSET_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_REPLY_ATTACHMENTS: usize = 5;
pub const MAX_IMAGE_EDGE: u32 = 2048;
pub const MAX_ASSET_CHUNKS_PER_GET: usize = 8;
pub const MAX_REQUEST_IMAGES: usize = 20;
/// Longest session title a request carries (characters). The push copy is cut shorter.
pub const SESSION_TITLE_MAX_CHARS: usize = 120;

pub const PUBLISH_RATE_PER_SEC: f64 = 6.0;
pub const PUBLISH_BURST: f64 = 10.0;
pub const WATCH_MIN_INTERVAL_MS: u64 = 2_000;
pub const WATCH_FRAME_MAX_BYTES: usize = 8 * 1024;
pub const MAX_LIVE_WATCH_PER_PHONE: usize = 2;
pub const MAX_FOLLOW: usize = 5;
/// Todos (docs/specs/ios-todos.md §5): the most pending / finished todos one `todo.list` carries,
/// the most project rows one `todo.projects` carries, and the longest todo text a phone may send.
pub const TODO_MAX_PENDING: usize = 300;
pub const TODO_MAX_HISTORY: usize = 100;
pub const TODO_MAX_PROJECTS: usize = 200;
pub const TODO_TEXT_MAX_CHARS: usize = 4000;
pub const KEEPALIVE_SECS: u64 = 30;
pub const PUSH_FALLBACK_SECS: u64 = 3;
pub const HANDLE_REFRESH_DAYS: i64 = 90;
pub const PAIR_TICKET_SECS: i64 = 300;
pub const PAIR_DONE_TIMEOUT_SECS: i64 = 120;
/// Replay window: messages whose `ts` differs from the receiver's clock by more are dropped.
pub const CLOCK_SKEW_MS: i64 = 15 * 60 * 1000;
pub const DEDUP_WINDOW_COUNT: usize = 2048;
pub const DEDUP_WINDOW_SECS: i64 = 600;
pub const JOURNAL_MAX_ENTRIES: usize = 5000;
pub const JOURNAL_MAX_AGE_DAYS: i64 = 30;
pub const PUSH_PREVIEW_MAX_CHARS: usize = 120;
/// The session title in a push is cut shorter than in the app: a notification title is one line.
pub const PUSH_SESSION_MAX_CHARS: usize = 60;

// ---------------------------------------------------------------- topics

pub fn topic_down(user: &str, phone_id: &str) -> String {
    format!("ah/{user}/d/{phone_id}")
}

pub fn topic_up(user: &str, mac_id: &str) -> String {
    format!("ah/{user}/u/{mac_id}")
}

pub fn topic_presence(user: &str, device_id: &str) -> String {
    format!("ah/{user}/s/{device_id}")
}

pub fn topic_presence_filter(user: &str) -> String {
    format!("ah/{user}/s/+")
}

pub fn topic_pair(user: &str, ticket_id: &str) -> String {
    format!("ah/{user}/p/{ticket_id}")
}

/// Device id of a presence topic `ah/{user}/s/{device}`.
pub fn presence_device(user: &str, topic: &str) -> Option<String> {
    let rest = topic.strip_prefix(&format!("ah/{user}/s/"))?;
    (!rest.is_empty() && !rest.contains('/')).then(|| rest.to_string())
}

/// Broker usernames become a topic level, so they must not contain wildcards or separators.
pub fn is_valid_broker_user(user: &str) -> bool {
    !user.is_empty() && user.len() <= 64 && user.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Retained presence message (plaintext; carries no content).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    /// 1 online, 0 offline.
    pub o: u8,
    pub t: i64,
    /// `mac` or `phone`.
    pub r: String,
    /// Phone is in the foreground.
    #[serde(default)]
    pub fg: u8,
    /// Mac gateway epoch (changes on every start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e: Option<String>,
}

// ---------------------------------------------------------------- message types

pub mod t {
    pub const PAIR_REQUEST: &str = "pair.request";
    pub const PAIR_PENDING: &str = "pair.pending";
    pub const PAIR_REJECT: &str = "pair.reject";
    pub const PAIR_ACCEPT: &str = "pair.accept";
    pub const PAIR_DONE: &str = "pair.done";

    pub const HELLO: &str = "hello";
    pub const HELLO_ACK: &str = "hello.ack";
    pub const ERROR: &str = "error";
    pub const RESYNC: &str = "resync";
    pub const PING: &str = "ping";
    pub const PONG: &str = "pong";
    pub const MAC_INFO: &str = "mac.info";
    pub const UNPAIR: &str = "unpair";
    pub const UNPAIRED: &str = "unpaired";
    pub const PUSH_REGISTER: &str = "push.register";
    pub const PUSH_STATE: &str = "push.state";

    pub const INBOX_SNAPSHOT: &str = "inbox.snapshot";
    pub const REQUEST_NEW: &str = "request.new";
    pub const REQUEST_GET: &str = "request.get";
    pub const REQUEST_FULL: &str = "request.full";
    pub const REQUEST_ACK: &str = "request.ack";
    pub const REQUEST_SEEN: &str = "request.seen";
    pub const REQUEST_RESOLVED: &str = "request.resolved";
    pub const ANSWER_SUBMIT: &str = "answer.submit";
    pub const ANSWER_ACK: &str = "answer.ack";
    pub const CONFIRM_DECIDE: &str = "confirm.decide";
    pub const CONFIRM_ACK: &str = "confirm.ack";

    pub const AGENTS_SNAPSHOT: &str = "agents.snapshot";
    pub const AGENTS_DELTA: &str = "agents.delta";
    pub const FOLLOW_SET: &str = "follow.set";
    pub const FOLLOW_STATE: &str = "follow.state";
    pub const FOLLOW_EVENT: &str = "follow.event";
    pub const WATCH_OPEN: &str = "watch.open";
    pub const WATCH_CLOSE: &str = "watch.close";
    pub const WATCH_FRAME: &str = "watch.frame";
    pub const WATCH_CLOSED: &str = "watch.closed";
    pub const INTERJECT_SEND: &str = "interject.send";
    pub const INTERJECT_ACK: &str = "interject.ack";
    pub const INTERJECT_CLEAR: &str = "interject.clear";
    pub const INTERJECT_QUERY: &str = "interject.query";
    pub const INTERJECT_STATE: &str = "interject.state";
    pub const INTERJECT_CONSUMED: &str = "interject.consumed";

    pub const TASK_OPTIONS_GET: &str = "task.options.get";
    pub const TASK_OPTIONS: &str = "task.options";
    pub const TASK_TODOS_GET: &str = "task.todos.get";
    pub const TASK_TODOS: &str = "task.todos";
    pub const TASK_CREATE: &str = "task.create";
    pub const TASK_STATUS: &str = "task.status";
    pub const SLOT_TAKE: &str = "slot.take";
    pub const SLOT_STATE: &str = "slot.state";

    pub const TODO_PROJECTS_GET: &str = "todo.projects.get";
    pub const TODO_PROJECTS: &str = "todo.projects";
    pub const TODO_LIST_GET: &str = "todo.list.get";
    pub const TODO_LIST: &str = "todo.list";
    pub const TODO_OP: &str = "todo.op";
    pub const TODO_ACK: &str = "todo.ack";
    pub const TODO_WATCH: &str = "todo.watch";
    pub const TODO_CHANGED: &str = "todo.changed";

    pub const ASSET_GET: &str = "asset.get";
    pub const ASSET_CHUNK: &str = "asset.chunk";
    pub const ASSET_ERROR: &str = "asset.error";
    pub const UPLOAD_BEGIN: &str = "upload.begin";
    pub const UPLOAD_CHUNK: &str = "upload.chunk";
    pub const UPLOAD_ACK: &str = "upload.ack";
    pub const UPLOAD_CANCEL: &str = "upload.cancel";
}

/// A typed message body with its wire type string.
pub trait Msg: Serialize {
    const TYPE: &'static str;

    fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("message body serializes")
    }
}

macro_rules! msg {
    ($ty:ty, $name:expr) => {
        impl Msg for $ty {
            const TYPE: &'static str = $name;
        }
    };
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

// ---------------------------------------------------------------- shared shapes

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Caps {
    pub new_task: bool,
    pub interject: bool,
    pub watch: bool,
    /// The Mac serves `todo.*` (docs/specs/ios-todos.md I7). Older Macs do not send it.
    #[serde(default)]
    pub todos: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub active: bool,
    pub auto_activation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef {
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRef {
    pub key: String,
    pub name: String,
    /// The project's repository as todo sync names it (normalized git remote, e.g. `github.com/acme/app`;
    /// the same value as `TodoProjectRow::repo`). Only the Agent list carries it, and only while todo sync is
    /// on: it is how the phone tells which Agents work in a project it lists in the Todos tab, whichever
    /// Mac and whichever worktree they are in (docs/specs/ios-todos.md I18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

// ---------------------------------------------------------------- pairing (plan §3.3)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub phone_id: String,
    pub name: String,
    /// Phone ephemeral X25519 public key, base64url.
    pub ek: String,
    pub app_ver: String,
    pub proto: u32,
}
msg!(PairRequest, t::PAIR_REQUEST);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairPending {
    pub mac_id: String,
}
msg!(PairPending, t::PAIR_PENDING);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairReject {
    pub reason: String,
}
msg!(PairReject, t::PAIR_REJECT);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairAccept {
    pub mac_id: String,
    pub mac_name: String,
    pub caps: Caps,
}
msg!(PairAccept, t::PAIR_ACCEPT);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairDone {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}
msg!(PairDone, t::PAIR_DONE);

// ---------------------------------------------------------------- handshake and sync (plan §3.4)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub nonce: String,
    pub proto: u32,
    pub app_ver: String,
    pub device_name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub lifecycle_results: bool,
}
msg!(Hello, t::HELLO);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloAck {
    pub nonce: String,
    /// Gateway epoch (16 hex chars); changes on every Mac gateway start.
    pub epoch: String,
    pub mac_name: String,
    pub caps: Caps,
    pub slot: SlotInfo,
}
msg!(HelloAck, t::HELLO_ACK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Message id or client message id the error refers to.
    #[serde(rename = "ref")]
    pub reference: String,
    /// `clock_skew`, `unknown_type`, `not_paired`, `rate_limited`, `internal`.
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
msg!(ErrorBody, t::ERROR);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resync {
    /// `inbox`, `agents`, `follow` or `slot`.
    pub stream: String,
}
msg!(Resync, t::RESYNC);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    pub t: i64,
}
msg!(Ping, t::PING);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pong {
    pub t: i64,
}
msg!(Pong, t::PONG);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacInfo {
    pub mac_name: String,
}
msg!(MacInfo, t::MAC_INFO);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Unpair {}
msg!(Unpair, t::UNPAIR);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Unpaired {}
msg!(Unpaired, t::UNPAIRED);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRegister {
    pub handle: String,
    pub env: String,
    pub expires_at: i64,
}
msg!(PushRegister, t::PUSH_REGISTER);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushState {
    /// `ok`, `missing`, `gone` or `error`.
    pub status: String,
}
msg!(PushState, t::PUSH_STATE);

// ---------------------------------------------------------------- requests

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRef {
    pub asset_id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub is_image: bool,
    /// Base64 JPEG thumbnail (<= 24 KB) for images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub too_large: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionView {
    pub text: String,
    pub recommended: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub todo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionView {
    pub text: String,
    pub options: Vec<OptionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub text: String,
    pub files: Vec<AssetRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskView {
    pub is_markdown: bool,
    pub message: MessageView,
    pub questions: Vec<QuestionView>,
    pub select_only: bool,
    pub single: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub whats_next: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmFieldView {
    pub id: String,
    pub label: String,
    pub value: String,
    /// `text`, `path` or `timestamp`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmDetailView {
    pub summary: String,
    pub body_md: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmChoiceView {
    pub wire_index: usize,
    pub label: String,
    pub description: String,
    /// Action role (`default`, `primary`, `danger`, ...), lowercase.
    pub role: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmInputView {
    pub id: String,
    pub label: String,
    pub placeholder: String,
    pub max_chars: usize,
    /// Choice index that makes the input visible; `None` means always visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when_index: Option<usize>,
    /// Whether a non-empty value is required when the input is visible.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmView {
    pub title: String,
    pub context: Vec<ConfirmFieldView>,
    pub detail: ConfirmDetailView,
    pub choices: Vec<ConfirmChoiceView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ConfirmInputView>,
    /// Wire index that dismissing the confirmation maps to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismiss_index: Option<usize>,
    pub submit_label: String,
    /// Unix milliseconds.
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestView {
    pub id: String,
    /// `ask` or `confirm`.
    pub kind: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub agent: AgentRef,
    pub project: ProjectRef,
    /// Title of the Agent session that asks (its saved name, else its first message), as the Mac
    /// knew it at delivery; later renames are not followed. A single line of at most
    /// `SESSION_TITLE_MAX_CHARS` characters. Additive: older phones ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    pub source: String,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<AskView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<ConfirmView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub id: String,
    pub kind: String,
    pub project: ProjectRef,
    pub agent: AgentRef,
    /// The same title as `RequestView::session_title`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    pub created_at: i64,
    pub preview: String,
    pub content_hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxSnapshot {
    pub epoch: String,
    pub rev: u64,
    pub items: Vec<RequestSummary>,
}
msg!(InboxSnapshot, t::INBOX_SNAPSHOT);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestNew {
    pub rev: u64,
    pub request: RequestView,
    /// `false` when only the summary-level content fits; fetch the rest with `request.get`.
    pub full: bool,
}
msg!(RequestNew, t::REQUEST_NEW);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestGet {
    pub request_id: String,
}
msg!(RequestGet, t::REQUEST_GET);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestFull {
    pub request: RequestView,
}
msg!(RequestFull, t::REQUEST_FULL);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestAck {
    pub request_id: String,
}
msg!(RequestAck, t::REQUEST_ACK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSeen {
    pub request_id: String,
}
msg!(RequestSeen, t::REQUEST_SEEN);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestResolved {
    pub rev: u64,
    pub request_id: String,
    /// `answered`, `cancelled`, `expired` or `answeredElsewhere`.
    pub how: String,
    /// Channel display name that answered (for `answeredElsewhere`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}
msg!(RequestResolved, t::REQUEST_RESOLVED);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerItem {
    /// Question index.
    pub q: usize,
    /// Selected option indices.
    #[serde(default)]
    pub sel: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Upload ids of reply images.
    #[serde(default)]
    pub images: Vec<String>,
    /// Upload ids of reply files.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerSubmit {
    pub client_msg_id: String,
    pub request_id: String,
    /// `send` or `cancel`.
    pub action: String,
    #[serde(default)]
    pub answers: Vec<AnswerItem>,
}
msg!(AnswerSubmit, t::ANSWER_SUBMIT);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerAck {
    pub client_msg_id: String,
    pub request_id: String,
    /// `accepted`, `lost`, `expired` or `invalid`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
msg!(AnswerAck, t::ANSWER_ACK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmDecide {
    pub client_msg_id: String,
    pub request_id: String,
    /// Chosen wire index; `None` with `dismiss` for the dismiss action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_index: Option<usize>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dismiss: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}
msg!(ConfirmDecide, t::CONFIRM_DECIDE);

/// Same shape as [`AnswerAck`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmAck {
    pub client_msg_id: String,
    pub request_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
msg!(ConfirmAck, t::CONFIRM_ACK);

// ---------------------------------------------------------------- agents, follow, watch, interject

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanInterject {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    /// `{kind}:{sessionId}`.
    pub key: String,
    pub kind: String,
    pub kind_label: String,
    pub session_id: String,
    pub title: String,
    pub project: ProjectRef,
    /// `working`, `idle`, `waiting` or `ended`.
    pub state: String,
    /// Unix milliseconds.
    pub started_at: i64,
    pub last_activity_at: i64,
    pub active_elapsed_secs: u64,
    pub turn_steps: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_preview: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pending_interject: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub last_turn_failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_result: Option<crate::agents::result::TurnResult>,
    pub can_interject: CanInterject,
    pub followed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsSnapshot {
    pub epoch: String,
    pub rev: u64,
    pub agents: Vec<AgentView>,
}
msg!(AgentsSnapshot, t::AGENTS_SNAPSHOT);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsDelta {
    pub rev: u64,
    #[serde(default)]
    pub upsert: Vec<AgentView>,
    #[serde(default)]
    pub remove: Vec<String>,
}
msg!(AgentsDelta, t::AGENTS_DELTA);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowSet {
    pub client_msg_id: String,
    pub agent_key: String,
    pub on: bool,
}
msg!(FollowSet, t::FOLLOW_SET);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowState {
    pub rev: u64,
    pub keys: Vec<String>,
    /// Set when the last `follow.set` was refused (`limit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
msg!(FollowState, t::FOLLOW_STATE);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowEvent {
    pub agent_key: String,
    /// `done` or `failed` (Macs before `docs/specs/ios-follow-narrowing.md` also sent `waiting`).
    pub kind: String,
    pub title: String,
    pub project: String,
    pub preview: String,
}
msg!(FollowEvent, t::FOLLOW_EVENT);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchOpen {
    pub agent_key: String,
}
msg!(WatchOpen, t::WATCH_OPEN);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchClose {
    pub agent_key: String,
}
msg!(WatchClose, t::WATCH_CLOSE);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchFrame {
    pub agent_key: String,
    pub seq: u64,
    pub frame: Value,
}
msg!(WatchFrame, t::WATCH_FRAME);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchClosed {
    pub agent_key: String,
    /// `ended`, `limit` or `notFound`.
    pub reason: String,
}
msg!(WatchClosed, t::WATCH_CLOSED);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectSend {
    pub client_msg_id: String,
    pub agent_key: String,
    pub text: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_result_id: Option<String>,
}
msg!(InterjectSend, t::INTERJECT_SEND);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectAck {
    pub client_msg_id: String,
    pub agent_key: String,
    /// `queued`, `delivered` or `rejected`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub pending: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_registered: Option<bool>,
}
msg!(InterjectAck, t::INTERJECT_ACK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectClear {
    pub client_msg_id: String,
    pub agent_key: String,
}
msg!(InterjectClear, t::INTERJECT_CLEAR);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectQuery {
    pub agent_key: String,
}
msg!(InterjectQuery, t::INTERJECT_QUERY);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectState {
    pub agent_key: String,
    pub pending: u64,
    /// Full text of the queued interjections (for recall), when requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}
msg!(InterjectState, t::INTERJECT_STATE);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectConsumed {
    pub agent_key: String,
}
msg!(InterjectConsumed, t::INTERJECT_CONSUMED);

// ---------------------------------------------------------------- tasks and slot

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TaskOptionsGet {}
msg!(TaskOptionsGet, t::TASK_OPTIONS_GET);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOption {
    pub id: String,
    pub label: String,
    pub subtitle: String,
    pub pinned: bool,
    pub last_used_at: i64,
}

/// One model an Agent offers for a new task (docs/specs/ios-task-model.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_supported: Option<bool>,
    /// What `task.create` sends back as `model`.
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Reasoning efforts this model can run with; absent = no effort row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
}

/// What "Default" stands for: the model the Agent uses when `task.create` names none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DefaultModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Efforts the default model can run with; absent = no effort row for "Default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_control: Option<bool>,
    pub kind: String,
    pub label: String,
    pub ready: bool,
    pub subtitle: String,
    pub permission_selectable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The models the phone may pick besides "Default"; absent when none is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<DefaultModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments_supported: Option<bool>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    /// `ask`, `agentDefault` or `yolo`.
    pub permission_mode: String,
    pub workspaces: Vec<WorkspaceOption>,
    pub agents: Vec<AgentOption>,
}
msg!(TaskOptions, t::TASK_OPTIONS);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskTodosGet {
    pub workspace_id: String,
}
msg!(TaskTodosGet, t::TASK_TODOS_GET);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoOption {
    pub id: String,
    pub text: String,
    pub auto: bool,
    pub attachments: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskTodos {
    pub workspace_id: String,
    pub todos: Vec<TodoOption>,
    pub overflow: u64,
}
msg!(TaskTodos, t::TASK_TODOS);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCreate {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<crate::agents::model_catalog::TaskSpeed>,
    pub client_msg_id: String,
    pub workspace_id: String,
    pub agent_kind: String,
    /// `default` or `yolo`.
    pub permission: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    pub text: String,
    /// A model id from `task.options`; absent = the Agent's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// A reasoning effort offered for that model (or for "Default"); absent = the Agent's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}
msg!(TaskCreate, t::TASK_CREATE);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub client_msg_id: String,
    /// `accepted`, `launching`, `launched`, `watching`, `unmatched` or `failed`.
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_key: Option<String>,
}
msg!(TaskStatus, t::TASK_STATUS);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotTake {
    pub client_msg_id: String,
}
msg!(SlotTake, t::SLOT_TAKE);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotState {
    pub rev: u64,
    pub active: bool,
    pub auto_activation: bool,
}
msg!(SlotState, t::SLOT_STATE);

// ---------------------------------------------------------------- todos (docs/specs/ios-todos.md §5)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TodoProjectsGet {}
msg!(TodoProjectsGet, t::TODO_PROJECTS_GET);

/// One project of the Mac's todo view. The phone never sees a path: `id` is a hash of the
/// project key and `hint` tells same-named projects apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoProjectRow {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Normalized git remote of a synced project, e.g. `github.com/acme/app`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub pending: u64,
    pub history: u64,
    /// `local` (sync is off), `synced`, `cloudOnly` or `localOnly`.
    pub scope: String,
    /// `withTodos`, `recent`, `synced`, `empty` or `localOnly`.
    pub section: String,
    /// `working` or `idle`: an Agent is live in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Names of the Macs that have a directory for a synced project.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoSyncDevice {
    pub name: String,
    pub last_sync_ms: u64,
    pub this_mac: bool,
}

/// Facts about iCloud todo sync on the Mac; the phone derives its status line from them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoSyncInfo {
    pub syncing: bool,
    /// `accountChanged`, `zoneMissing`, `notSignedIn` or `quota`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<String>,
    /// `running`, `notRunning`, `incompatible` or `unsupported`.
    pub helper: String,
    pub unsynced: u64,
    pub last_pull_ms: u64,
    pub last_push_ms: u64,
    pub devices: Vec<TodoSyncDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoProjects {
    pub rev: u64,
    /// `local`, or `icloud` when this Mac syncs its todos.
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<TodoSyncInfo>,
    pub projects: Vec<TodoProjectRow>,
    /// More projects existed than [`TODO_MAX_PROJECTS`].
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}
msg!(TodoProjects, t::TODO_PROJECTS);

impl TodoProjects {
    /// No projects, sync off.
    pub fn empty() -> Self {
        Self {
            rev: 0,
            mode: "local".to_string(),
            sync: None,
            projects: Vec::new(),
            truncated: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoListGet {
    pub id: String,
}
msg!(TodoListGet, t::TODO_LIST_GET);

/// "N attachments, on that Mac": attachments never sync (todo-sync T6), only this hint does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoAttHint {
    pub n: u64,
    pub dev: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    pub text: String,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto: bool,
    /// Attachments this Mac holds for the todo (the phone only shows the count).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub att: u64,
    /// Name of the Mac that created the todo, when it is not the one answering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub att_on: Option<TodoAttHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoDone {
    pub id: String,
    pub text: String,
    pub created_at_ms: u64,
    pub done_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub att: u64,
    /// Name of the Mac that finished the todo, when it is not the one answering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_on: Option<String>,
}

/// The todos of one project. Sent in answer to `todo.list.get` and inside `todo.ack`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoList {
    pub id: String,
    /// The project id still names a project of this Mac (everything below is empty when not).
    pub found: bool,
    pub rev: u64,
    pub name: String,
    /// Whether the ⚡ flag means anything here (false for a project without a directory on the Mac).
    pub can_auto: bool,
    /// Newest last, like the queue; at most [`TODO_MAX_PENDING`].
    pub pending: Vec<TodoItem>,
    pub pending_total: u64,
    /// Newest first; at most [`TODO_MAX_HISTORY`].
    pub history: Vec<TodoDone>,
    pub history_total: u64,
}
msg!(TodoList, t::TODO_LIST);

impl TodoList {
    /// The answer for an id that names no project of this Mac.
    pub fn missing(id: &str, rev: u64) -> Self {
        Self {
            id: id.to_string(),
            found: false,
            rev,
            name: String::new(),
            can_auto: false,
            pending: Vec::new(),
            pending_total: 0,
            history: Vec::new(),
            history_total: 0,
        }
    }

    /// Drops what does not fit: the history first, then the tail of the queue (the todos the
    /// Agent takes last), halving each time. Returns `false` when nothing is left to drop.
    pub fn shrink(&mut self) -> bool {
        if !self.history.is_empty() {
            self.history.truncate(self.history.len() / 2);
            true
        } else if self.pending.len() > 1 {
            self.pending.truncate(self.pending.len() / 2);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoOp {
    pub client_msg_id: String,
    /// Project id.
    pub id: String,
    /// `add`, `edit`, `complete`, `restore`, `remove`, `reorder` or `auto`.
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    /// The pending ids in their new order (`reorder`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}
msg!(TodoOp, t::TODO_OP);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoAck {
    pub client_msg_id: String,
    /// `ok`, `notFound`, `rejected` or `failed`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The id of the todo `add` created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    /// The project as it is after the operation (also after a refusal, so the phone catches up).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<TodoList>,
}
msg!(TodoAck, t::TODO_ACK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoWatch {
    pub on: bool,
}
msg!(TodoWatch, t::TODO_WATCH);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoChanged {
    pub rev: u64,
}
msg!(TodoChanged, t::TODO_CHANGED);

// ---------------------------------------------------------------- assets and uploads

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetGet {
    pub asset_id: String,
    pub from: u64,
    pub count: u64,
}
msg!(AssetGet, t::ASSET_GET);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetChunk {
    pub asset_id: String,
    pub index: u64,
    /// Number of chunks of the whole file.
    pub total: u64,
    /// Size of the whole file in bytes.
    pub size: u64,
    /// SHA-256 (hex) of the whole file, present on the first chunk only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Base64 chunk data.
    pub data: String,
}
msg!(AssetChunk, t::ASSET_CHUNK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetError {
    pub asset_id: String,
    /// `notFound`, `tooLarge` or `internal`.
    pub reason: String,
}
msg!(AssetError, t::ASSET_ERROR);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadBegin {
    pub upload_id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub chunks: u64,
    pub sha256: String,
    pub scope: UploadScope,
}
msg!(UploadBegin, t::UPLOAD_BEGIN);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadChunk {
    pub upload_id: String,
    pub index: u64,
    /// Base64 chunk data.
    pub data: String,
}
msg!(UploadChunk, t::UPLOAD_CHUNK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadAck {
    pub upload_id: String,
    pub next_index: u64,
    /// `ok`, `complete` or `error`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
msg!(UploadAck, t::UPLOAD_ACK);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadCancel {
    pub upload_id: String,
}
msg!(UploadCancel, t::UPLOAD_CANCEL);

// ---------------------------------------------------------------- registry

macro_rules! registry {
    ($($ty:ty),* $(,)?) => {
        /// Every message type of the catalog (pairing messages included).
        pub const ALL_TYPES: &[&str] = &[$(<$ty as Msg>::TYPE),*];

        /// Strictly decodes `body` as the message type `ty` and encodes it back; `None` for an
        /// unknown type. Used to validate peer input and by the cross-language fixtures.
        pub fn normalize(ty: &str, body: &Value) -> Option<Result<Value, serde_json::Error>> {
            match ty {
                $(<$ty as Msg>::TYPE => Some(
                    serde_json::from_value::<$ty>(body.clone())
                        .and_then(|msg| serde_json::to_value(&msg)),
                ),)*
                _ => None,
            }
        }
    };
}

registry!(
    PairRequest,
    PairPending,
    PairReject,
    PairAccept,
    PairDone,
    Hello,
    HelloAck,
    ErrorBody,
    Resync,
    Ping,
    Pong,
    MacInfo,
    Unpair,
    Unpaired,
    PushRegister,
    PushState,
    InboxSnapshot,
    RequestNew,
    RequestGet,
    RequestFull,
    RequestAck,
    RequestSeen,
    RequestResolved,
    AnswerSubmit,
    AnswerAck,
    ConfirmDecide,
    ConfirmAck,
    AgentsSnapshot,
    AgentsDelta,
    FollowSet,
    FollowState,
    FollowEvent,
    WatchOpen,
    WatchClose,
    WatchFrame,
    WatchClosed,
    InterjectSend,
    InterjectAck,
    InterjectClear,
    InterjectQuery,
    InterjectState,
    InterjectConsumed,
    TaskOptionsGet,
    TaskOptions,
    TaskTodosGet,
    TaskTodos,
    TaskCreate,
    TaskStatus,
    SlotTake,
    SlotState,
    TodoProjectsGet,
    TodoProjects,
    TodoListGet,
    TodoList,
    TodoOp,
    TodoAck,
    TodoWatch,
    TodoChanged,
    AssetGet,
    AssetChunk,
    AssetError,
    UploadBegin,
    UploadChunk,
    UploadAck,
    UploadCancel,
);

// ---------------------------------------------------------------- push plaintext (plan §4.8)

/// Plaintext carried (encrypted) inside an APNs push; the notification extension decrypts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushPlain {
    /// `ask`, `confirm`, `follow.done`, `follow.failed` or `resolved` (older Macs also sent
    /// `follow.waiting`).
    pub k: String,
    /// Request id (or agent key for follow events).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rid: Option<String>,
    /// Mac display name.
    pub mac: String,
    pub agent: String,
    pub project: String,
    /// Title of the Agent session behind a request (cut short for a notification).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Preview text (<= 120 characters).
    pub text: String,
    pub ts: i64,
}

/// Request body of the relay's `/v1/push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayPush {
    pub handle: String,
    /// `alert` or `replace`.
    pub kind: String,
    pub loc_key: String,
    pub thread_id: String,
    pub collapse_id: String,
    pub ah: super::envelope::Envelope,
}

// ---------------------------------------------------------------- pairing code

/// `ahpair1.` + base64url(JSON): everything a phone needs to pair (also shown as a QR code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCode {
    pub v: u32,
    /// Broker host.
    pub h: String,
    /// TLS port.
    pub p: u16,
    /// WSS port.
    pub w: u16,
    /// Broker username.
    pub u: String,
    /// Broker password.
    pub pw: String,
    /// Relay base URL.
    pub r: String,
    /// Mac id.
    pub m: String,
    /// Mac display name.
    pub mn: String,
    /// Ticket id.
    pub t: String,
    /// Ticket secret, base64url.
    pub ts: String,
    /// Mac ephemeral public key, base64url.
    pub ek: String,
    /// Expiry, unix seconds.
    pub x: i64,
}

pub const PAIR_CODE_PREFIX: &str = "ahpair1.";

impl PairCode {
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("pair code serializes");
        format!("{PAIR_CODE_PREFIX}{}", super::crypto::b64u(&json))
    }

    pub fn decode(text: &str) -> Option<Self> {
        let body = text.trim().strip_prefix(PAIR_CODE_PREFIX)?;
        serde_json::from_slice(&super::crypto::b64u_decode(body)?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_have_the_documented_layout() {
        assert_eq!(topic_down("u1", "p"), "ah/u1/d/p");
        assert_eq!(topic_up("u1", "m"), "ah/u1/u/m");
        assert_eq!(topic_presence("u1", "x"), "ah/u1/s/x");
        assert_eq!(topic_presence_filter("u1"), "ah/u1/s/+");
        assert_eq!(topic_pair("u1", "t"), "ah/u1/p/t");
        assert_eq!(presence_device("u1", "ah/u1/s/x").as_deref(), Some("x"));
        assert_eq!(presence_device("u1", "ah/u2/s/x"), None);
        assert_eq!(presence_device("u1", "ah/u1/s/x/y"), None);
        assert_eq!(presence_device("u1", "ah/u1/s/"), None);
    }

    #[test]
    fn broker_user_rejects_wildcards() {
        assert!(is_valid_broker_user("askhuman01"));
        for bad in ["", "+", "#", "+/#", "a/b", "a b", "名字"] {
            assert!(!is_valid_broker_user(bad), "{bad}");
        }
    }

    #[test]
    fn pair_code_round_trip() {
        let code = PairCode {
            v: PROTO,
            h: "broker.example".into(),
            p: 8883,
            w: 8084,
            u: "user1".into(),
            pw: "pw".into(),
            r: "https://relay.example".into(),
            m: "m".repeat(26),
            mn: "Home Mac".into(),
            t: "t".repeat(26),
            ts: super::super::crypto::b64u(&[1u8; 32]),
            ek: super::super::crypto::b64u(&[2u8; 32]),
            x: 1_800_000_000,
        };
        let text = code.encode();
        assert!(text.starts_with("ahpair1."));
        assert_eq!(PairCode::decode(&text), Some(code.clone()));
        assert_eq!(PairCode::decode(&format!("  {text}\n")), Some(code));
        assert_eq!(PairCode::decode("ahpair1.@@@"), None);
        assert_eq!(PairCode::decode("other"), None);
    }

    #[test]
    fn optional_fields_are_omitted_on_the_wire() {
        let ack = AnswerAck {
            client_msg_id: "c".into(),
            request_id: "r".into(),
            status: "accepted".into(),
            reason: None,
        };
        assert_eq!(
            ack.to_value(),
            serde_json::json!({"clientMsgId": "c", "requestId": "r", "status": "accepted"})
        );
        assert_eq!(AnswerAck::TYPE, "answer.ack");
    }

    #[test]
    fn the_session_title_is_optional_in_both_directions() {
        let plain = |session: Option<&str>| PushPlain {
            k: "ask".into(),
            rid: Some("r".into()),
            mac: "Home".into(),
            agent: "Claude Code".into(),
            project: "relay".into(),
            session: session.map(str::to_string),
            text: "Pick one".into(),
            ts: 1,
        };
        // Absent when there is none: a phone or Mac that predates the field sees what it always saw.
        let without = serde_json::to_value(plain(None)).unwrap();
        assert!(without.get("session").is_none(), "{without}");
        let with = serde_json::to_value(plain(Some("Fix the flaky test"))).unwrap();
        assert_eq!(with["session"], "Fix the flaky test");

        // And a message from a peer that predates it still reads.
        let old: PushPlain = serde_json::from_value(serde_json::json!({
            "k": "ask", "mac": "Home", "agent": "A", "project": "p", "text": "t", "ts": 1
        }))
        .unwrap();
        assert_eq!(old.session, None);
        let old: RequestSummary = serde_json::from_value(serde_json::json!({
            "id": "r", "kind": "ask", "project": {"key": "k", "name": "p"},
            "agent": {"kind": "claude", "label": "Claude Code"}, "createdAt": 1, "preview": "t",
            "contentHash": "h", "size": 1
        }))
        .unwrap();
        assert_eq!(old.session_title, None);
        let summary = serde_json::to_value(RequestSummary {
            session_title: Some("Fix it".into()),
            ..old
        })
        .unwrap();
        assert_eq!(summary["sessionTitle"], "Fix it");
    }
}
