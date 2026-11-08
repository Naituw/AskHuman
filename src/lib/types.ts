import type { CodexLaunchStatus, CursorLaunchStatus } from "./ipc";

export type OutputFormat = "text" | "json";

export interface AskRequest {
  id: string;
  isMarkdown: boolean;
  message: MessagePrompt;
  questions: Question[];
  /** 严格选择：禁用自由文本 / 回复附件，只能勾选预设项（全局）。 */
  selectOnly: boolean;
  /** 单选：每题恰好一个选择（默认多选，全局）。 */
  single: boolean;
  /** 结果输出格式（全局；仅影响 CLI 输出，弹窗不关心）。 */
  outputFormat: OutputFormat;
  /** whats-next 提问（spec todo-whats-next D2/D7）：待办已是问题选项，折叠待办区只留增删。 */
  whatsNext?: boolean;
  /** Set when the question reached the user although away mode is on (docs/specs/special-replies.md D21). */
  escalated?: EscalationReason;
  /** False for Stop confirmations, failure cards, native Codex questions and development relays (N10); absent = true. */
  quickReply?: boolean;
  /**
   * The quick replies the menu offers on this question, in order (docs/specs/special-replies.md D27): the user's
   * own list or the factory one as it was when the question was created. Absent or empty = none (What's next,
   * strict-choice questions, anything without the menu).
   */
  quickReplies?: string[];
  /** The Agent's text may carry notes the user writes on passages of it (docs/specs/message-annotations.md); absent = false. */
  annotatable?: boolean;
  /** One of the setup wizard's own test questions (docs/specs/onboarding.md D11): the popup shows no setup pointer on it; absent = false. */
  setupProbe?: boolean;
}

/** Why a question came through away mode: the Agent asked for it, it asked repeatedly, or the user asked it to ask. */
export type EscalationReason = "asked" | "repeated" | "requested";

// ===== Away mode and per-session mute (docs/specs/special-replies.md; mirrors `daemon::away`) =====

/** A session's own setting; a session without one follows the global switch. */
export type AwaySessionMode = "away" | "mute" | "normal";

/** What a surface can ask for one session (`follow` removes the override). */
export type AwaySessionSetting = "follow" | AwaySessionMode;

export interface AwayGlobalView {
  sinceMs: number;
  note?: string | null;
  /** Automatic replies handed out since the switch went on. */
  replies: number;
  /** The same count per session key (`sid:<id>`); the empty key collects callers without a session. */
  repliesBySession?: Record<string, number>;
}

export interface AwaySessionView {
  /** `sid:<agent session id>` or `mcp:<instance id>`. */
  sessionKey: string;
  mode: AwaySessionMode;
  sinceMs: number;
  note?: string | null;
  replies: number;
}

/** The state as every surface shows it. */
export interface AwaySnapshot {
  /** Present while the global switch is on. */
  global?: AwayGlobalView | null;
  sessions: AwaySessionView[];
  /** Escalated questions that are waiting for an answer right now. */
  escalatedWaiting: number;
}

/** One Agent session in the away capsule's expanded card (mirrors `app::away_capsule::CapsuleRow`). */
export interface CapsuleRow {
  /** Agent family: claude / codex / cursor / grok. */
  kind: string;
  title: string;
  replies: number;
}

/** What the desktop capsule and its card show (mirrors `app::away_capsule::CapsuleView`). */
export interface CapsuleView {
  /** `all` while the global switch is on; `sessions` when only single sessions are away. */
  scope: "all" | "sessions";
  sinceMs: number;
  /** Sessions that are away on their own (scope `sessions`). */
  sessions: number;
  /** Their keys: "end away mode" puts each back to following the global switch. */
  sessionKeys: string[];
  replies: number;
  /** Escalated questions waiting for an answer. */
  waiting: number;
  rows: CapsuleRow[];
  /** Rows that did not fit the card. */
  more: number;
  note?: string | null;
}

/** The capsule window's state: the view (`null` = nothing to show) and whether the card is "welcome back". */
export interface CapsuleState {
  view: CapsuleView | null;
  welcome: boolean;
}

/** What the capsule window asks for when it opens. */
export interface CapsuleInit {
  state: CapsuleState;
  theme: ThemeMode;
  lang: string;
}

/** One state-changing request (`daemon::away::AwayOp`). */
export type AwayOp =
  | { op: "state" }
  | { op: "start"; note?: string | null }
  | { op: "end" }
  | { op: "session"; sessionKey: string; setting: AwaySessionSetting; note?: string | null };

export type ConfirmFieldKind = "text" | "path" | "timestamp";
export type ConfirmActionRole = "primary" | "default" | "destructive";

export interface ConfirmField {
  id: string;
  label: string;
  value: string;
  kind: ConfirmFieldKind;
}

export interface ConfirmDetail {
  summary: string;
  bodyMd: string;
}

/** 前缀档位元数据（D51）：同 group 的 choice 是同一动作的不同泛化档位。 */
export interface ChoiceVariant {
  group: string;
  level: number;
  levelLabel: string;
  /** Exact token chunk added at this level; old daemons fall back to levelLabel. */
  segmentLabel?: string;
  recommended: boolean;
}

export interface ConfirmChoice {
  id: string;
  label: string;
  description: string;
  role: ConfirmActionRole;
  variant?: ChoiceVariant | null;
}

export interface ConfirmInput {
  id: string;
  visibleWhenActionId: string;
  label: string;
  placeholder: string;
  maxChars: number;
}

export interface ConfirmPresentation {
  type: "singleSelectSubmit";
  input?: ConfirmInput | null;
  submitLabel: string;
  defaultActionId?: string | null;
}

export interface ConfirmRequest {
  id: string;
  title: string;
  context: ConfirmField[];
  detail: ConfirmDetail;
  choices: ConfirmChoice[];
  presentation: ConfirmPresentation;
  dismissActionId: string;
  createdAtMs: number;
  expiresAtMs: number;
}

export type SnapshotStatus =
  | "payload_only"
  | "snapshot_ready"
  | "new_file"
  | "protected_path"
  | "timeout"
  | "too_large"
  | "too_many_files"
  | "non_utf8"
  | "not_regular_file"
  | "unreadable"
  | "source_mismatch"
  | "unsupported";

export type PermissionFileChangeKind =
  | "added"
  | "modified"
  | "deleted"
  | "moved"
  | "proposed";

export type PermissionDiffLineKind = "context" | "add" | "delete" | "meta";

export interface PermissionDiffLine {
  kind: PermissionDiffLineKind;
  oldLine?: number | null;
  newLine?: number | null;
  text: string;
}

export interface PermissionDiffHunk {
  oldStart?: number | null;
  newStart?: number | null;
  header: string;
  lines: PermissionDiffLine[];
}

export interface PermissionDiffFile {
  changeKind: PermissionFileChangeKind;
  oldPath?: string | null;
  newPath: string;
  snapshotStatus: SnapshotStatus;
  hunks: PermissionDiffHunk[];
  additions: number;
  deletions: number;
  omittedHunks: number;
  omittedLines: number;
}

export interface PermissionDiffModel {
  requestId: string;
  snapshotStatus: SnapshotStatus;
  snapshotAtMs?: number | null;
  files: PermissionDiffFile[];
  totalFiles: number;
  additions: number;
  deletions: number;
  omittedFiles: number;
  omittedHunks: number;
  omittedLines: number;
  truncated: boolean;
}

export interface PatchLine {
  kind: PermissionDiffLineKind;
  text: string;
}

export interface PatchHunk {
  header: string;
  lines: PatchLine[];
}

export interface PatchFile {
  kind: "add" | "update" | "delete" | "move";
  oldPath?: string | null;
  newPath: string;
  hunks: PatchHunk[];
}

export type PermissionEditOperation =
  | {
      type: "textReplace";
      path: string;
      oldText: string;
      newText: string;
      replaceAll: boolean;
    }
  | { type: "wholeFileWrite"; path: string; content: string }
  | { type: "patchSet"; files: PatchFile[] }
  | { type: "unsupported"; reason: "notebook_edit" | "invalid_payload" };

export interface PermissionEditIntent {
  agentKind: string;
  nativeTool: string;
  workspace: string;
  operation: PermissionEditOperation;
  initialDiff?: PermissionDiffModel | null;
}

export type InteractionRequest =
  | { type: "ask"; request: AskRequest }
  | { type: "confirm"; request: ConfirmRequest };

export interface MessagePrompt {
  development?: DevVerificationInfo | null;
  text: string;
  files: FileAttachment[];
}

export interface DevBuildInfo {
  version: number;
  worktree: string;
  binary: string;
  sourceFingerprint: string;
  binaryFingerprint: string;
  gitCommit: string | null;
  createdAtMs: number;
}

export interface DevVerificationInfo {
  diagnostics?: { home: string; endpoint: string; reason: string } | null;
  id: string;
  worktree: string;
  build: DevBuildInfo;
  delivery: "local" | "remote" | "channels";
  relay: boolean;
  testProfile?: string | null;
}

export interface DevWorkbenchStatus {
  systemBinary: string | null;
  worktrees: DevWorktreeInfo[];
  agents: DevAgentSupport[];
  instanceTarget: { kind: "system" | "dev" | "isolated"; home: string; binary: string } | null;
}

export interface DevWorktreeInfo {
  root: string;
  name: string;
  binary: string;
  home: string;
  enabled: boolean;
  build: DevBuildInfo | null;
  error: string | null;
  preferences: { delivery: "local" | "remote" | "channels" };
}

export interface DevAgentSupport {
  terminalAvailable?: boolean;
  launchReason?: string | null;
  kind: string;
  supported: boolean;
  executable: string | null;
  reason: string | null;
}

/** 单个预定义选项：文本 + 是否为提问方（AI）的推荐答案。 */
export interface OptionItem {
  text: string;
  recommended: boolean;
  /** whats-next / Stop 卡待办 chip 对应的待办条目 id（spec todo-whats-next D2/D5）。 */
  todoId?: string | null;
  todoText?: string | null;
  todoAttachments?: TodoAttachmentSnapshot[];
}

export type TodoAttachmentStorage = "managed" | "reference";

/** One attachment owned by or referenced from a project todo. */
export interface TodoAttachmentView {
  id: string;
  name: string;
  size: number;
  isImage: boolean;
  sourcePath: string;
  /** Effective path opened or delivered to an Agent. */
  path: string;
  storage: TodoAttachmentStorage;
  available: boolean;
}

export interface TodoAttachmentSnapshot {
  id: string;
  name: string;
  path: string;
  sourcePath: string;
  storage: TodoAttachmentStorage;
}

/** 项目级待办条目（spec todo-whats-next D1）。 */
export interface TodoEntry {
  id: string;
  text: string;
  createdAtMs: number;
  /** Agent family that added the todo through the CLI; absent for human-created and legacy rows. */
  agentKind?: string | null;
  /** 自动执行：whats-next 时不提问直接派发（后端 auto=false 时省略该字段）。 */
  auto?: boolean;
  attachments?: TodoAttachmentView[];
}

/** 已执行的历史待办（仅执行出队进历史）。 */
export interface TodoDoneEntry {
  id: string;
  text: string;
  createdAtMs: number;
  /** Preserved Agent origin from the pending todo. */
  agentKind?: string | null;
  doneAtMs: number;
  attachments?: TodoAttachmentView[];
}

/** 待办窗口项目选择器候选（spec todo-whats-next D9）。 */
export interface TodoProjectInfo {
  /** 项目 key（git 根路径）。 */
  key: string;
  /** 显示名（basename）。 */
  name: string;
  /** 该项目当前待办条数。 */
  count: number;
  /**
   * 选择器分组：
   * - `withTodos`：当前有待办的项目
   * - `recent`：最近工作过的项目（活跃 Agent / workspace；不含已在 withTodos 出现的 key）
   */
  section: "withTodos" | "recent" | string;
}

/** 待办窗口 init 负载。 */
export interface TodosInit {
  theme: ThemeMode;
  lang: string;
  /** 与弹窗一致的提交快捷键（添加待办）。 */
  popupSubmitKey: PopupSubmitKey;
  /** Whether a supported platform terminal is available for creating Agent tasks. */
  newTaskSupported: boolean;
  /** 性能埋点是否开启（宿主进程带 `ASKHUMAN_PERF_ID`）；前端据此决定是否上报 perf 标记。 */
  perf?: boolean;
  /** The user's home directory, for the `~/...` path abbreviation in the project header. */
  home?: string;
}

/** One project's pending todos in the "All todos" view of the todos window. */
export interface TodoGroup {
  /** Project key. */
  key: string;
  entries: TodoEntry[];
}

/** 新建任务窗口 init 负载（spec gui-agent-task-launch）。 */
export interface NewTaskInit {
  theme: ThemeMode;
  lang: string;
  /** 与弹窗一致的提交快捷键（⌘↵ 启动任务）。 */
  popupSubmitKey: PopupSubmitKey;
  /** `agentTasks.permissionPrompt`（G6）。 */
  permissionPrompt: "ask" | "agent-default" | "yolo" | string;
}

/** Launch settings offered by the selected Agent, shared by all desktop new-task entry points. */
export interface NewTaskOptions {
  listed: boolean;
  models: TaskModelOption[];
  default: TaskModelDefault;
  speedControl: boolean;
  maxControl: boolean;
}

export interface TaskModelDefault {
  label?: string | null;
  efforts: string[];
  defaultEffort?: string | null;
  maxRequired?: boolean | null;
}

export interface TaskModelOption extends TaskModelDefault {
  id: string;
  label: string;
  note?: string | null;
  fastSupported?: boolean | null;
}

/** 新建任务窗口的项目下拉候选。 */
export interface NewTaskProject {
  /** workspace 路径（canonical cwd）或待办项目 git 根。 */
  path: string;
  /** 显示名（basename）。 */
  label: string;
  /** 置顶 workspace（列表已按置顶排序；展示加 ★）。 */
  pinned: boolean;
  /** `workspace`（最近 workspace 索引）或 `todos`（仅存在于待办存储）。 */
  source: "workspace" | "todos" | string;
}

export interface ForkTaskSource {
  sessionId: string;
  seq: number;
  kind: AgentKind;
  title: string;
  cwd: string;
  state: AgentRunState;
  forkedFromSessionId?: string | null;
  /** Runtime-probed native Fork capability for this active source session. */
  forkReady?: boolean;
}

export interface ForkTaskInit {
  theme: ThemeMode;
  lang: string;
  popupSubmitKey: PopupSubmitKey;
  permissionPrompt: "ask" | "agent-default" | "yolo" | string;
  source: ForkTaskSource;
}

export interface Question {
  message: string;
  predefinedOptions: OptionItem[];
}

export interface FileAttachment {
  path: string;
  name: string;
  size: number;
  isImage: boolean;
}

export interface ImageAttachment {
  data: string;
  mediaType: string;
  filename?: string | null;
}

// ===== Files dropped onto an answer popup (src-tauri/src/popup_drop.rs; docs/specs/popup-inbox-collapse.md §6.9) =====

/** Where a dropped file lives, as far as the system treats the place specially. A label from the backend, never a path. */
export type DropPlace = "desktop" | "documents" | "downloads" | "icloud" | "volume" | "other";
export type DropReadReason = "permission_denied" | "not_found" | "other";
/** What `popup_drop_read_image` rejects with when a dropped picture cannot be read. */
export interface DropReadError {
  reason: DropReadReason;
  where: DropPlace;
  /** The system's own words for the failure. */
  message: string;
}
export type DropLogAction = "enter" | "leave" | "drop" | "error";
/** What became of a drop; the backend rejects a label that is not one of these. */
export type DropLogOutcome =
  | "added" | "partial" | "read_failed" | "duplicates" | "no_paths" | "on_preview"
  | "not_accepting" | "busy" | "own_drag_out" | "no_receiver" | "request_gone" | "no_target";
export type DropLogTarget = "card" | "body" | "row" | "preview" | "none";
/**
 * One drag event for the diagnostic log (`daemon.log`). The backend writes labels and counts only: `paths` are counted
 * and classed by place there, and neither they nor any file name reach the log.
 */
export interface DropLogEntry {
  action: DropLogAction;
  requestId?: string | null;
  view?: "expanded" | "collapsed";
  /** The native event the drag was first seen by: `over` when the `enter` was missed. */
  from?: "enter" | "over";
  paths?: string[];
  outcome?: DropLogOutcome;
  target?: DropLogTarget;
  /** The files are an attachment one of the window's own requests was dragged out of (R17), not files from another app. */
  own?: boolean;
  added?: number;
  images?: number;
  files?: number;
  duplicates?: number;
  failed?: { path: string; reason: DropReadReason }[];
}

export type ThemeMode = "system" | "light" | "dark";

export type PopupAnimation = "none" | "document" | "alert";

export type WindowEffect = "glass" | "blur" | "solid";

/** Daemon-owned navigation metadata; forms pull their own request by ID. */
export interface PopupInboxRequest {
  callerCwd?: string | null;
  requestId: string;
  sequence: number;
  interaction: InteractionRequest;
  project: string;
  source: string;
  lang: string;
  agentKind?: string | null;
  agentConsoleSessionId?: string | null;
  agentSessionTitle?: string | null;
  createdAtMs: number;
  kind?: "ask" | "permission" | "stop";
}
export interface PopupInboxSnapshot {
  requests: PopupInboxRequest[];
  recovered: boolean;
  presented?: boolean;
  focusedRequestId?: string | null;
}

export interface PopupInit {
  callerCwd?: string | null;
  /** Current interaction. A prewarmed popup returns null until assigned. */
  interaction: InteractionRequest | null;
  /** Local-popup-only native edit intent for permission confirmations. */
  popupEdit?: PermissionEditIntent | null;
  theme: ThemeMode;
  alwaysOnTop: boolean;
  sourceName: string;
  /** 来源 workspace 完整路径（hover 显示）；空表示未知，前端隐藏该元素。 */
  project: string;
  /** workspace 目录名（标题区展示）。 */
  projectName: string;
  /** 发起本次提问的 agent 家族（claude/codex/cursor/grok）；空表示未识别，不显示 agent badge。 */
  agentKind?: string | null;
  /** 发起本次提问的 agent 进程 pid；「聚焦终端」用。 */
  agentPid?: number | null;
  /** daemon 严格匹配到活动 Agent 记录的会话 ID；有值才显示 Agent Window 快捷入口。 */
  agentConsoleSessionId?: string | null;
  /** Trusted native session identity, independent from lifecycle tracking. */
  agentSessionId?: string | null;
  agentSessionTitle?: string | null;
  /** 界面语言原始值（auto/en/zh）；弹窗据此 applyLanguage，免再走 get_settings()。 */
  language?: string;
  /** 语音识别语言（BCP-47，如 zh-CN；auto 跟随系统）。 */
  speechLanguage?: string;
  /** 语音输入快捷键（规范串如 cmd+d；空串=关闭）。 */
  speechShortcut?: string;
  /** 提交快捷键：cmdEnter（默认）或 enter。 */
  popupSubmitKey?: PopupSubmitKey;
  /** 多问题弹窗纵向同时显示所有问题（默认开；关 = 旧版一次一题）。 */
  verticalQuestions?: boolean;
  /** 性能埋点是否开启（helper 收到 ASKHUMAN_PERF_ID）；前端据此决定是否上报 perf 标记。 */
  perf?: boolean;
  /** 性能测试：画完首帧后自动取消弹窗（仅 harness 用）。 */
  perfAutodismiss?: boolean;
  /** Whether this helper started as a hidden prewarmed popup before it adopted the interaction. */
  warm?: boolean;
  /** 提问创建时刻（epoch 毫秒）：弹窗据此显示相对时间（几秒/分钟/小时前），超过一天显示绝对时间。0=未知。 */
  createdAtMs?: number;
}

export interface QuestionAnswer {
  selectedOptions: string[];
  userInput: string;
  images: ImageAttachment[];
  files: string[];
  /** 折叠待办区选中的待办条目 id（spec todo-whats-next D7）：文本已并入 userInput，id 供后端出队。 */
  todoIds?: string[];
  todoSelections?: Array<{
    id: string;
    attachments: TodoAttachmentSnapshot[];
  }>;
}

/** A one-shot instruction that travels with an answer (the quick-reply menu; docs/specs/special-replies.md D19). */
export interface PopupDirective {
  kind: "away" | "mute";
  /** Mute is always about the asking session; away may cover every Agent or only this session. */
  scope: "global" | "session";
  note?: string;
}

/** A note the user wrote on a passage of the Agent's text (docs/specs/message-annotations.md). */
export interface UserAnnotation {
  /** The passage as the user saw it: rendered text folded to one line, middle-truncated at 2000 characters. */
  quote: string;
  note: string;
}

export interface PopupSubmission {
  answers: QuestionAnswer[];
  directive?: PopupDirective;
  /** In reading order; left out when there are none. The daemon cleans and caps them again. */
  annotations?: UserAnnotation[];
}

export type ChannelAction = "send" | "cancel";

/** One question's recorded answer in history (paths only, no base64). */
export interface HistoryAnswer {
  selectedOptions: string[];
  userInput?: string | null;
  /** Saved image file paths (best-effort to display). */
  images: string[];
  /** Reply file paths (best-effort to display). */
  files: string[];
}

/** One recorded reply (one per request: the winning terminal result). */
export interface HistoryEntry {
  id: string;
  timestampMs: number;
  project: string;
  source: string;
  /** Caller agent family (claude/codex/cursor/grok); absent on legacy entries. */
  agentKind?: string | null;
  /** Native Agent conversation/session id; absent on legacy or unbound entries. */
  agentSessionId?: string | null;
  /** AskHuman MCP server process id; a fallback partition when no native session is known. */
  mcpInstanceId?: string | null;
  /** Channel that submitted / cancelled: popup / dingding / feishu / telegram. */
  channel: string;
  action: ChannelAction;
  isMarkdown: boolean;
  message: MessagePrompt;
  questions: Question[];
  answers: HistoryAnswer[];
  /** Notes written on the Agent's text with the answer; absent on entries recorded without any. */
  annotations?: UserAnnotation[];
}

/** Aggregated project info for the history window's project picker. */
export interface ProjectInfo {
  key: string;
  name: string;
  count: number;
  lastMs: number;
}

/** Trustworthy session partition used by the history filter. */
export type HistorySessionRef =
  | { type: "agent"; agentKind: string; sessionId: string }
  | { type: "mcp"; project: string; instanceId: string }
  | { type: "unbound" };

/** One aggregated session option derived from the currently loaded history entries. */
export interface HistorySessionGroup {
  token: string;
  ref: HistorySessionRef;
  count: number;
  lastMs: number;
}

/** Batch title lookup sent only for exact native Agent sessions. */
export interface HistorySessionTitleRequest {
  token: string;
  agentKind: string;
  sessionId: string;
}

export interface HistorySessionTitleResult {
  token: string;
  title: string;
}

/** Popup-originated initial/retarget filter for the global history window. */
export type HistoryOpenTarget =
  | { type: "agent"; agentKind: string; sessionId: string }
  | { type: "mcp"; project: string; instanceId: string }
  /** Every reply of one channel: the away-mode "view the record" link (channel `away` / `mute`). */
  | { type: "channel"; channel: string };

export interface HistoryOpenRequest {
  all: boolean;
  project?: string | null;
  target?: HistoryOpenTarget | null;
}

/** History window init payload. */
export interface HistoryInit {
  theme: ThemeMode;
  /** 界面语言（已解析为 en/zh）；历史窗口据此 applyLanguage。 */
  lang: string;
  project: string;
  projectName: string;
}

/** Agent 控制台（状态窗口）init 负载。 */
export interface AgentsInit {
  theme: ThemeMode;
  lang: string;
  /** 与弹窗一致的提交快捷键（输入框 ⌘↵ 发送）。 */
  popupSubmitKey: PopupSubmitKey;
  /** Whether a supported platform terminal is available for creating Agent tasks. */
  newTaskSupported: boolean;
}

export type AgentKind = "claude" | "codex" | "cursor" | "grok" | "pi" | "antigravity";

/** Lifecycle preference and artifact state inside an Agent integration. */
export interface LifecycleStatus {
  enabled: boolean;
  preferenceConfigured: boolean;
  installed: boolean;
  outdated: boolean;
  supported: boolean;
  needsUpdate: boolean;
  cleanupRequired: boolean;
}

export type AgentRunState = "working" | "idle" | "ended";

/** 单个被追踪 agent（一条 session）的快照记录。 */
export interface AgentRecord {
  desktop?: {connected: boolean; error?: string};
  /** 稳定数字编号（当前 daemon 生命周期内单调、不复用）；供 IM `/status <编号>` 寻址。 */
  seq?: number;
  kind: AgentKind;
  sessionId: string;
  pid?: number | null;
  title?: string | null;
  cwd?: string | null;
  /** Direct parent session for a branch created by AskHuman's native Fork flow. */
  forkedFromSessionId?: string | null;
  /** AskHuman-created terminal task UUID; required for exact Windows Terminal focus. */
  launchId?: string | null;
  /** Runtime-probed native Fork capability for this active source session. */
  forkReady?: boolean;
  startedAt: number;
  lastActivity: number;
  state: AgentRunState;
  endedAt?: number | null;
  /** 所在终端类型（apple-terminal/iterm2/windows-terminal/vscode/…）；用于聚焦按钮显隐。 */
  terminal?: string | null;
  /** 实时「当前工具」（hook 上报，仅 snapshot、不落盘）：`{name, object?, at}`。GUI 暂不消费。 */
  currentTool?: { name: string; object?: string | null; at: number } | null;
  /** 有待送达的插话消息（daemon 注入；驱动「待送达」徽标与撤回按钮）。 */
  pendingInterject?: boolean;
  /** 在途 AskHuman 提问的请求 id（daemon 注入，spec gui-agent-console C7/R2）：
   *  🙋 徽标 + 「去回答」精确聚焦对应弹窗。 */
  waitingRequestId?: string | null;
  /** 在途提问的摘要预览（等待横幅展示；可缺省）。 */
  waitingPreview?: string | null;
  /** 累计有效工作时长（秒，registry snapshot 含当前区间的生效总值）。 */
  activeElapsedSecs?: number | null;
  /** 控制台可以把消息直接送进这个聊天（daemon 注入，只给订阅快照；目前是 Cursor Desktop 的空闲 /
   *  已结束聊天）：它们没有会有下一次工具调用的 hook，消息不能排队，只能立刻发。 */
  canMessage?: boolean;
}

// ===== Agent 控制台（spec gui-agent-console）=====

/** 焦点会话详情帧的工具步（daemon `frame_detail_json`）。 */
export interface DetailStep {
  /** 结构化类别：run/read/write/other（本地化由前端完成）。 */
  kind: "run" | "read" | "write" | "other" | string;
  /** kind=other 时的原始工具名。 */
  name?: string | null;
  object?: string | null;
  state: "running" | "done" | "failed" | string;
}

/** 焦点会话详情帧的 TODO 条目。 */
export interface DetailTodo {
  content: string;
  state: "pending" | "inProgress" | "completed" | string;
}

/** 完整会话事件（console_transcript 输出，tagged；spec gui-agent-console C14）。 */
export type TranscriptEventJson =
  | { type: "user"; text: string; at?: number | null; atLabel?: string | null }
  | { type: "assistant"; text: string; at?: number | null; atLabel?: string | null }
  | { type: "thinking"; text: string; at?: number | null; atLabel?: string | null }
  | {
      type: "tool";
      label: string;
      object?: string | null;
      isError: boolean;
      resultSummary?: string | null;
      at?: number | null;
      atLabel?: string | null;
    }
  | {
      type: "ask";
      kind?: "ask" | "whatsNext";
      message: string;
      questions: { text: string; answer?: string | null }[];
      at?: number | null;
      atLabel?: string | null;
    }
  | { type: "meta"; text: string };

/** 完整会话一页（`[start, start+events.length)` 窗口）。 */
export interface TranscriptPage {
  events: TranscriptEventJson[];
  start: number;
  total: number;
  truncatedHead: boolean;
  partial: boolean;
}

/** 项目未暂存变更统计的一行（console_diff_stat；spec gui-agent-console C15）。 */
export interface DiffFileStat {
  path: string;
  /** M 修改 / D 删除 / A 新增 / B 二进制。 */
  kind: "M" | "D" | "A" | "B" | string;
  adds: number;
  dels: number;
}

export interface DiffStatPage {
  /** git 根（stage 调用沿用）。 */
  root: string;
  files: DiffFileStat[];
}

/** 单文件 hunk 视图（console_diff_file）。 */
export interface DiffFileView {
  path: string;
  kind: string;
  skipped: boolean;
  skipReason?: string | null;
  lines: { kind: "add" | "del" | "context" | "header" | string; text: string }[];
}

export interface LatestAgentText {
  id: string;
  revision: string;
  kind: "assistant" | "thinking";
  text: string;
  truncated: boolean;
  at?: number | null;
}

/** 焦点会话详情帧（tagged，daemon 签名变化才推；spec gui-agent-console C8/R5）。 */
export interface AgentDetailFrame {
  type: "watchFrame" | string;
  sessionId: string;
  seq: number;
  kindLabel: string;
  phase: "working" | "idle" | "waiting" | "ended" | string;
  title?: string | null;
  project?: string | null;
  text?: string | null;
  latestText?: LatestAgentText | null;
  steps: DetailStep[];
  stepsOmitted: number;
  todos: DetailTodo[];
  activeElapsedSecs?: number | null;
  at?: number | null;
}

/** 插话 composer 窗口 init 负载。 */
export interface InterjectInit {
  theme: ThemeMode;
  lang: string;
  /** 待送达全文（预填编辑；空 = 无待送达）。 */
  text: string;
  /** 待送达条数。 */
  entries: number;
  /** Flattened pending attachment references. */
  attachments: InterjectAttachment[];
}

export interface InterjectAttachment extends FileAttachment {
  available: boolean;
}

export interface InterjectPending {
  text: string;
  entries: number;
  attachments: InterjectAttachment[];
}

export type UiLanguage = "auto" | "en" | "zh";

/** Popup/Confirm submit key mode (mirrors Rust `PopupSubmitKey`). */
export type PopupSubmitKey = "cmdEnter" | "enter";

/** Global collaboration style for agent prompts (mirrors Rust `CollaborationStyle`). */
export type CollaborationStyle = "aligned" | "autonomous" | "custom";

export interface GeneralConfig {
  theme: ThemeMode;
  /** 界面语言：auto（跟随系统）/ en / zh。回退英文。 */
  language: UiLanguage;
  alwaysOnTop: boolean;
  appearAnimation: PopupAnimation;
  windowEffect: WindowEffect;
  /** 语音识别语言（BCP-47，如 "zh-CN"）；"auto" 跟随系统首选语言。 */
  speechLanguage: string;
  /** 语音输入快捷键（弹窗内）。规范串如 "cmd+d"；空串表示关闭。 */
  speechShortcut: string;
  /**
   * Popup/Confirm submit shortcut:
   * - `cmdEnter`: ⌘/Ctrl+Enter submits (default); bare Enter newlines
   * - `enter`: bare Enter submits; any modifier+Enter newlines
   */
  popupSubmitKey: PopupSubmitKey;
  /** 协作风格：对齐 / 自主 / 自定义。 */
  collaborationStyle: CollaborationStyle;
  /** 自定义协作风格正文；空则回退对齐默认。 */
  collaborationStyleCustomText: string;
  /** 回复历史保留条数上限。默认 200；0 = 停止新增记录（但保留旧记录）。 */
  historyLimit: number;
  /** 待办执行历史保留条数（每项目）。默认 100；0 = 停止新增记录（保留旧历史）。 */
  todoHistoryLimit: number;
  /** Built-in popup sound. Empty disables it; macOS stores a name, other desktops use a toggle. */
  popupSound: string;
  /** Menu bar / system-tray status icon mode (off/active/always). */
  menuBarIcon: MenuBarIconMode;
  /** Popup pre-warm (faster popups by keeping one mounted, hidden helper ready). Default true. */
  popupPrewarm: boolean;
  /** Daemon lifecycle: activity（按需起+空闲退出）/ keepalive（常驻+开机自启）。 */
  daemonLifecycle: DaemonLifecycleMode;
  /** Away mode on the desktop (docs/specs/special-replies.md D14, D22, D23, D26). */
  away: AwayConfig;
  /**
   * The user's own quick replies (docs/specs/special-replies.md D27); `null` = never edited, so the factory
   * list in the interface language applies. At most 10, each one line.
   */
  quickReplies: string[] | null;
}

/** The factory quick replies in both interface languages (`quick_reply_defaults`). */
export interface QuickReplyDefaults {
  en: string[];
  zh: string[];
}

/** Away-mode surfaces and timing (mirrors Rust `AwayConfig`); the mode itself lives in the daemon. */
export interface AwayConfig {
  /** Keep the always-on-top capsule on screen while away (macOS only). */
  capsule: boolean;
  /** Expand the capsule into "welcome back" when the user returns after being idle (macOS only). */
  welcomeBack: boolean;
  /** Seconds the Mac must be idle before an escalated question is pushed to the phone; 0 = at once. */
  pushAfterSecs: number;
}

/** Menu bar / tray status icon mode (mirrors Rust `MenuBarIconMode`). */
export type MenuBarIconMode = "off" | "active" | "always";

/** Daemon lifecycle mode (mirrors Rust `DaemonLifecycleMode`). */
export type DaemonLifecycleMode = "activity" | "keepalive";

/** Popup sound support: kind="named" with names, "toggle", or "none". */
export interface PopupSoundSupport {
  kind: "named" | "toggle" | "none";
  names: string[];
}

export interface PopupChannelConfig {
  enabled: boolean;
  windowMode: "merged" | "independent";
  width: number;
  height: number;
  rememberSize: boolean;
  previewWidth: number;
  sidebarWidth: number;
}

export interface TelegramChannelConfig {
  enabled: boolean;
  botToken: string;
  chatId: string;
  apiBaseUrl: string;
}

export interface DingTalkChannelConfig {
  enabled: boolean;
  clientId: string;
  clientSecret: string;
  userId: string;
  cardTemplateId: string;
  confirmCardTemplateId: string;
  permissionConfirmCardTemplateId: string;
  inlineSmallText: boolean;
  convertTextToDocx: boolean;
}

export interface FeishuChannelConfig {
  enabled: boolean;
  appId: string;
  appSecret: string;
  openId: string;
  baseUrl: string;
}

export interface SlackChannelConfig {
  enabled: boolean;
  botToken: string;
  appToken: string;
  userId: string;
}

/** iOS 渠道（实验功能，docs/specs/ios-channel.md）：自备 MQTT broker + 推送中继。 */
export interface IosChannelConfig {
  enabled: boolean;
  host: string;
  port: number;
  wsPort: number;
  username: string;
  /** 只在输入时存在；已保存的值不回传（见 `SecretsPresent.iosBrokerPassword`）。 */
  password: string;
  relayUrl: string;
  /** 本机在手机上的显示名；留空取主机名。 */
  macName: string;
  /**
   * 手机没在看某个问题时，Mac 键盘/鼠标空闲满这么多秒才推送通知（`0` = 问题一到就推）。
   * 问题本身随时可在手机 App 里看到，与此无关（docs/specs/ios-alert-timing.md）。
   */
  alertAfterSecs: number;
}

export interface ChannelsConfig {
  popup: PopupChannelConfig;
  telegram: TelegramChannelConfig;
  dingding: DingTalkChannelConfig;
  feishu: FeishuChannelConfig;
  slack: SlackChannelConfig;
  ios: IosChannelConfig;
  /** 「IM 渠道按需发送」开关（默认关；显式配置的用户设置保持原值）。 */
  autoActivation: boolean;
  /** 「自动结束 watch」——「按需发送」子开关（默认开，仅 autoActivation 开时生效）。 */
  autoEndWatch: boolean;
}

/** 实验性功能开关（默认隐藏；开启后显示「实验」Tab）。 */
export interface ExperimentalConfig {
  enabled: boolean;
  /** 多问题弹窗纵向同时显示所有问题（默认开；关 = 旧版一次一题）。 */
  verticalQuestions: boolean;
}

/** iCloud todo sync settings (the on/off switch is a marker file, not config; spec todo-sync §8). */
export interface TodoSyncConfig {
  /** This Mac's name in the synced device list; empty = the computer name. */
  deviceName: string;
  /** Poll interval of the sync helper in minutes (not shown in the UI). */
  pollMinutes: number;
}

/** What `todo_sync_status` returns (mirror of Rust `todo_sync::service::Status`). */
export interface TodoSyncStatus {
  /** macOS 14 or later, outside a Dev Instance. */
  supported: boolean;
  enabled: boolean;
  /** A tick is running right now. */
  syncing: boolean;
  deviceName: string;
  /** Why this Mac stopped syncing and waits for the user. */
  paused: "accountChanged" | "zoneMissing" | "notSignedIn" | "quota" | null;
  lastPullOkMs: number;
  lastPushOkMs: number;
  /** Entities with local changes the cloud has not acknowledged. */
  unsynced: number;
  todos: number;
  history: number;
  devices: { name: string; lastSyncMs: number; thisMac: boolean }[];
  helper: {
    state: "running" | "notRunning" | "incompatible" | "unsupported";
    version: string | null;
    environment: string | null;
    problem: string | null;
    installed: boolean;
    bundled?: boolean;
    needsApproval?: boolean;
    installationError?: string | null;
  };
  lastTick: {
    atMs: number;
    reason: string;
    outcome:
      | "ok"
      | "fresh"
      | "offline"
      | "timedOut"
      | "unavailable"
      | "paused"
      | "busy"
      | "disabled"
      | "error";
    message?: string;
    fetched: number;
    merged: number;
    pushed: number;
  } | null;
  /** `state.json` belongs to a newer build. */
  newerState: boolean;
}

/** One Mac in a synced project's list (mirror of Rust `DeviceRow`). */
export interface TodoSyncMac {
  name: string;
  lastSyncMs: number;
  thisMac: boolean;
}

/**
 * One synced project, merged across this Mac's directories (mirror of Rust `service::SyncProject`;
 * spec todo-sync §5.2, §9.4).
 */
export interface TodoSyncProject {
  /** Normalized git remote, e.g. `github.com/acme/app`. */
  identity: string;
  /** This Mac's directories of the project (project keys); empty = only the iCloud replica exists here. */
  keys: string[];
  /** The key the todos live under while no directory exists (`icloud:<identity>`). */
  cloudKey: string | null;
  /** Macs that have a directory for the project, this one first. */
  macs: TodoSyncMac[];
}

/** Where a todo came from / was finished (mirror of Rust `service::Origin`). */
export interface TodoSyncOrigin {
  /** The Mac's name; empty while its record has not arrived yet. */
  device: string;
  thisMac: boolean;
}

/** What `todo_sync_project_details` returns, all keyed by todo id (mirror of Rust `ProjectDetails`). */
export interface TodoSyncProjectDetails {
  /** Pending todos created on another Mac → that Mac's name. */
  createdOn: Record<string, string>;
  /** Finished todos → where they were finished. */
  doneOn: Record<string, TodoSyncOrigin>;
  /** Pending todos whose attachments live on another Mac. */
  attachments: Record<string, { count: number; device: string }>;
}

/** Outcome of the helper's login-item registration. */
export interface TodoSyncRepair {
  ok: boolean;
  needsApproval: boolean;
  message: string;
}

/** 权限确认相关全局设置（spec codex-permission-remember）。 */
export interface PermissionsConfig {
  /** Codex shell 宽松模式全局开关（D52）：非危险且可解析的 shell 命令自动放行。 */
  codexRelaxedShell: boolean;
}

export type AgentTaskPermission = "ask" | "agent-default" | "yolo";

/** How Max mode is handled for Cursor tasks (spec cursor-max-control §6). */
export interface CursorMaxConfig {
  /** `enforce-off`: keep Max off and refuse models that need it; `inherit`: do nothing. */
  policy: "enforce-off" | "inherit";
  /** The account's plan: detected from Cursor's data, or set by hand. */
  plan: "auto" | "legacy" | "token-based";
}

export interface AgentTasksConfig {
  enabled: boolean;
  permissionPrompt: AgentTaskPermission;
  cursorMax: CursorMaxConfig;
}

export interface AgentTaskWorkspace {
  path: string;
  label: string;
  lastUsedAt: number;
  agents: AgentKind[];
  pinned: boolean;
  hidden: boolean;
}

export interface AgentTaskReadiness {
  kind: AgentKind;
  label: string;
  command: string;
  executable: string | null;
  version: string | null;
  binaryReady: boolean;
  lifecycleReady: boolean;
  integrationReady: boolean;
  integrationMode: string;
  ready: boolean;
  /** Whether New Task / Fork can start this family; Settings and doctor still show all families. */
  launchSupported: boolean;
  /**
   * Whether a task can be started on this Agent right now — the one verdict the phone, IM, the
   * wizard and Settings all use. It also counts the routes `ready` ignores: Codex's Desktop App
   * and Cursor's Agents window. Absent only in older builds and test fixtures: read it through
   * `isStartable`, which falls back to `ready`.
   */
  startable?: boolean;
  /** Where a task would run when `startable`; `null` when it cannot start. */
  startRoute?: "desktop" | "terminal" | null;
  /** Codex and Cursor: the launch status behind `startable` (both routes, and why each is unavailable). */
  launch?: CodexLaunchStatus | CursorLaunchStatus | null;
  diagnostics: string[];
  /** Codex only: oldest version that can receive the protocol through its SessionStart hook. */
  protocolMinimumVersion?: string | null;
  /** `false` only for a Codex older than `protocolMinimumVersion`; informational. */
  protocolSupported?: boolean;
}

export interface AppConfig {
  codexDesktop?: {appPath: string; codexHome: string; launchPreference: "terminal" | "desktop"};
  cursorDesktop?: {suppressBackgroundCompletionFollowUp: boolean; launchPreference?: "terminal" | "desktop"};
  general: GeneralConfig;
  channels: ChannelsConfig;
  agentTasks: AgentTasksConfig;
  permissions: PermissionsConfig;
  experimental: ExperimentalConfig;
  todoSync?: TodoSyncConfig;
}

/** Whether each channel secret is currently stored (drives the "Saved" placeholder). */
export interface SecretsPresent {
  dingdingSecret: boolean;
  feishuSecret: boolean;
  telegramToken: boolean;
  slackBotToken: boolean;
  slackAppToken: boolean;
  iosBrokerPassword: boolean;
}

/** Settings payload: config with secrets blanked + per-secret presence flags. */
export interface SettingsPayload {
  everydayIntegrationsReadOnly?: boolean;
  config: AppConfig;
  secretsPresent: SecretsPresent;
}

/** 一条渠道故障摘要（R7，镜像 Rust `ipc::ChannelIssueInfo`）：出现即表示该渠道仍未恢复。 */
export interface ChannelIssue {
  /** 渠道 id："telegram" / "dingding" / "feishu" / "slack"。 */
  channel: string;
  /** 错误文案（源语言英文，与 daemon.log 一致）。 */
  message: string;
  /** 首次出现的 Unix 毫秒时间戳。 */
  atMs: number;
}

// ===== Codex 权限授权管理面板（spec codex-permission-remember §6.3，镜像 Rust ipc 类型）=====

/** 一个对话的授权摘要（镜像 `permission_rules::SessionRuleSummary`）。 */
export interface PermissionSessionSummary {
  sessionId: string;
  ruleCount: number;
  fileExactCount: number;
  projectRoots: string[];
  fullDisk: boolean;
  shellCount: number;
  networkCount: number;
  mcpCount: number;
  /** 该会话 YOLO 模式开启中（D53）。 */
  yolo: boolean;
  lastUsedAtMs: number;
}

/** 面板分组：store 摘要 + registry 标题/项目名增强（可为空串）。 */
export interface PermissionSessionGroup {
  summary: PermissionSessionSummary;
  title: string;
  projectName: string;
}

export type PermissionRuleKind =
  | "fileExact"
  | "fileProject"
  | "fileDisk"
  | "mcpTool"
  | "networkHost"
  | "shellExact"
  | "shellPrefix"
  | "shellRelaxed"
  | "yolo";

/** 一条规则展示行（D48：原样键文本）。 */
export interface PermissionRuleInfo {
  kind: PermissionRuleKind;
  display: string;
  createdAtMs: number;
  lastUsedAtMs: number;
  expiresAtMs: number;
}

export type PermissionRulesOp =
  | { op: "summaries" }
  | { op: "sessionDetail"; sessionId: string }
  | { op: "globalDetail" }
  | { op: "resetSession"; sessionId: string }
  | { op: "resetGlobal" }
  | { op: "disableYolo"; sessionId: string };

export type PermissionRulesResult =
  | { kind: "summaries"; sessions: PermissionSessionGroup[]; globalCount: number }
  | { kind: "rules"; rules: PermissionRuleInfo[] }
  | { kind: "reset"; removed: number };

/** Per-secret edit intent sent on save. Secrets never round-trip through the config object. */
export type SecretAction =
  | { kind: "unchanged" }
  | { kind: "set"; value: string }
  | { kind: "clear" };

export interface SecretActions {
  dingdingSecret: SecretAction;
  feishuSecret: SecretAction;
  telegramToken: SecretAction;
  slackBotToken: SecretAction;
  slackAppToken: SecretAction;
  iosBrokerPassword: SecretAction;
}

/** 已配对的手机（镜像 Rust `ios::gateway::PhoneView`）。 */
export interface IosPhone {
  phoneId: string;
  name: string;
  aliasName?: string;
  /** `pendingConfirm` | `active` */
  state: string;
  online: boolean;
  foreground: boolean;
  createdAt: number;
  lastSeenAt: number;
  /** 推送登记状态：`ok` | `gone` | `error`；无登记时缺省。 */
  push?: string;
  /** `keychain` | `file` */
  keyBackend?: string;
}

/** 进行中的配对会话（镜像 Rust `PairingView`）。 */
export interface IosPairing {
  ticketId: string;
  /** 配对码（含 broker 密码）：只用于生成二维码 / 供用户复制。 */
  code: string;
  expiresAtMs: number;
  /** `waiting` | `pending`（手机已扫码，等待本机决定）| `accepted` */
  stage: string;
  phoneName?: string;
  /** 6 位确认码，仅 `pending` 阶段有。 */
  sas?: string;
}

/** Broker traffic of one month, counted on this Mac. */
export interface IosTraffic {
  /** `YYYY-MM` the counters belong to. */
  month: string;
  bytesIn: number;
  bytesOut: number;
  msgsIn: number;
  msgsOut: number;
}

export interface IosGatewayState {
  connected: boolean;
  /** `tls` | `wss` | `tcp` */
  via?: string;
  lastError?: string;
  macId: string;
  macName: string;
  phones: IosPhone[];
  pairing?: IosPairing;
  traffic?: IosTraffic;
  pendingRequests: number;
  /** Seconds since the last keyboard / mouse input on this Mac; absent when it cannot tell. */
  macIdleSecs?: number;
}

export type IosOp =
  | { op: "state" }
  | { op: "pairStart" }
  | { op: "pairCancel" }
  | { op: "pairDecision"; ticketId: string; allow: boolean }
  | { op: "revoke"; phoneId: string }
  | { op: "rename"; phoneId: string; name: string };

export type IosResult =
  | { kind: "state"; state: IosGatewayState | null }
  | { kind: "pairing"; ticketId: string; code: string; expiresAtMs: number }
  | { kind: "done"; found: boolean };

/** 粘贴 `ahconn1.` 连接串解析出的表单字段。 */
export interface IosConnectionFields {
  host: string;
  port: number;
  wsPort: number;
  username: string;
  password: string;
  relayUrl: string;
}

export interface HookStatus {
  installed: boolean;
  outdated: boolean;
  hooksJsonExists: boolean;
  supported: boolean;
}

export interface ClaudeHookStatus {
  installed: boolean;
  outdated: boolean;
  settingsExists: boolean;
  supported: boolean;
}

export type AgentId = "cursor" | "claude" | "codex" | "grok" | "pi" | "antigravity";

export interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseNotes: string;
  sourceUrl: string;
  isNpm: boolean;
  applyMode: UpdateApplyMode;
  manualCommand: string;
}

export type UpdateApplyMode = "automatic" | "manualDirect" | "manualNpm";

export interface PushedUpdateState {
  available: boolean;
  latestVersion: string;
  pending: boolean;
  applyMode: UpdateApplyMode;
}

/** 调用方 agent 的异步解析结果（方案5/b）：daemon walk 出家族 + pid 后经 `agent-resolved` 后推弹窗。 */
export interface PushedAgent {
  kind?: string | null;
  pid?: number | null;
  launchId?: string | null;
}

export interface RuleStatus {
  installed: boolean;
  outdated: boolean;
  path: string;
  supported: boolean;
}

/** Agent 集成模式（三态互斥）。 */
export type AgentMode = "none" | "cli" | "mcp";

/** 某家 Agent 的模式聚合状态（驱动设置页三态分段控件 + 产物清单）。 */
export interface AgentModeStatus {
  mode: AgentMode;
  needsUpdate: boolean;
  ruleNeedsUpdate: boolean;
  hookNeedsUpdate: boolean;
  mcpNeedsUpdate: boolean;
  rulePath: string;
  ruleInstalled: boolean;
  timeoutHookSupported: boolean;
  timeoutHookInstalled: boolean;
  timeoutHookNeedsUpdate: boolean;
  recoveryHookInstalled: boolean;
  permission: PermissionStatus;
  permissionNeedsUpdate: boolean;
  stop: StopStatus;
  delegated: DelegatedStatus;
  lifecycle: LifecycleStatus;
  askQuestion: AskQuestionStatus;
  mcpSupported: boolean;
  mcpConfigPath: string;
  mcpConfigInstalled: boolean;
  runtimeArtifactKind: "hook" | "extension";
  agentVersion: string | null;
  minimumVersion: string | null;
  versionSupported: boolean;
}

/** 接管 Claude 内置 AskUserQuestion 的开关状态（仅 Claude Code 支持）。 */
export interface AskQuestionStatus {
  supported: boolean;
  enabled: boolean;
  installed: boolean;
  outdated: boolean;
}

export interface StopStatus {
  supported: boolean;
  enabled: boolean;
  installed: boolean;
  outdated: boolean;
  otherHandlersDetected: boolean;
}

/** "Allow delegated runs (e.g. `codex exec`) to use AskHuman" preference. `enabled` = allowed. */
export interface DelegatedStatus {
  supported: boolean;
  enabled: boolean;
}

export interface PermissionStatus {
  supported: boolean;
  unsupportedReason: string | null;
  enabled: boolean;
  configured: boolean;
  outdated: boolean;
  needsUpdate: boolean;
  knownBlockedReason: string | null;
  otherHandlersDetected: boolean;
}

export interface TelegramTestArgs {
  botToken: string;
  chatId: string;
  apiBaseUrl: string;
}

export interface DingTalkTestArgs {
  clientId: string;
  clientSecret: string;
  userId: string;
}

export interface DingTalkDetectArgs {
  clientId: string;
  clientSecret: string;
}

export interface DingTalkWaitArgs {
  clientId: string;
  clientSecret: string;
  code: string;
}

export interface FeishuTestArgs {
  appId: string;
  appSecret: string;
  openId: string;
  baseUrl: string;
}

export interface FeishuDetectArgs {
  appId: string;
  appSecret: string;
  baseUrl: string;
}

export interface FeishuWaitArgs {
  appId: string;
  appSecret: string;
  baseUrl: string;
  code: string;
}

export interface SlackTestArgs {
  botToken: string;
  appToken: string;
  userId: string;
}

export interface SlackDetectArgs {
  botToken: string;
  appToken: string;
}

export interface SlackWaitArgs {
  botToken: string;
  appToken: string;
  code: string;
}

/** Exact-session, best-effort native facts shown only inside the Agent source card. */
export interface PopupAgentInfo {
  workspace?: string | null;
  model?: string | null;
  reasoning?: string | null;
  /** Cursor Desktop only: whether the session runs in Max mode, as its own record says. */
  maxMode?: boolean | null;
  context?: { usedTokens: number; windowTokens: number; remainingPercent: number; estimated: boolean } | null;
  reportedAt?: number | null;
}
export interface PopupProjectInfo {
  branch?: string | null;
  detached: boolean;
  worktree?: string | null;
  changes?: { total: number; staged: number; unstaged: number; untracked: number } | null;
}

// ===== Setup wizard (docs/specs/onboarding.md; mirrors the Rust `onboarding` module) =====

/** Which first-run questions are settled, and what this machine can offer (`setup_status`). */
export interface SetupStatus {
  /** At least one Agent is integrated, was skipped on purpose, or has already answered through AskHuman. */
  agentsDecided: boolean;
  /** An IM channel is on, or the remote-answering step was skipped. */
  remoteDecided: boolean;
  /** The GUI Host login item exists, or the user answered the background question. */
  backgroundDecided: boolean;
  loginItemInstalled: boolean;
  /** The platform has a login item this instance may manage (not a Dev Instance). */
  loginItemAvailable: boolean;
  trayAvailable: boolean;
  /** A Dev Instance must not change the everyday Agent integration. */
  everydayIntegrationsReadOnly: boolean;
  /** The wizard's phone-task offer was turned down ("Not now"): a rerun does not offer it again. */
  phoneTasksDeclined: boolean;
}

/** Why an Agent counts as installed on this machine. */
export interface SetupAgentSignals {
  cli: boolean;
  configDir: boolean;
  app: boolean;
}

/** One file the integration writes; `exists` = applying modifies it instead of creating it. */
export interface SetupPlannedFile {
  path: string;
  exists: boolean;
}

export interface SetupPlannedByMode {
  cli: SetupPlannedFile[];
  mcp: SetupPlannedFile[];
}

/** A mode the wizard can apply (it never uninstalls). */
export type SetupAgentMode = "cli" | "mcp";

/** Codex protocol delivery state; `untrusted` right after installing means "trust the hook on first launch". */
export type SetupCodexDelivery =
  | "notInstalled"
  | "trusted"
  | "untrusted"
  | "needsUpdate"
  | "unsupportedVersion";

/** One row of the wizard's Agent page (`setup_agents_scan`). */
export interface SetupAgentRow {
  id: AgentId;
  title: string;
  signals: SetupAgentSignals;
  detected: boolean;
  /** The integration mode currently on disk. */
  mode: AgentMode;
  needsUpdate: boolean;
  /** Why the state could not be read; the row is shown disabled. */
  statusError: string | null;
  supportsCli: boolean;
  supportsMcp: boolean;
  recommended: SetupAgentMode;
  delivery: SetupCodexDelivery | null;
  planned: SetupPlannedByMode;
}

export interface SetupSelection {
  agent: AgentId;
  mode: SetupAgentMode;
}

export interface SetupApplyResult {
  agent: AgentId;
  requested: SetupAgentMode;
  ok: boolean;
  error: string | null;
  /** The mode read back after the write. */
  modeAfter: AgentMode;
  delivery: SetupCodexDelivery | null;
}

/** Environment findings worth a card on the Agent page (`setup_env_check`). */
export interface SetupEnvReport {
  askhumanOnPath: "found" | "missing" | "unknown";
  pathFix: { command: string; dir: string; needsPathEdit: boolean } | null;
  transient: { path: string; kind: "temp" | "downloads" | "npx" } | null;
  installs: { message: string } | null;
}

/** One remote-answering channel's state (`setup_channel_status`). */
export interface SetupChannelStatus {
  id: string;
  enabled: boolean;
  configured: boolean;
  /** `null` while the daemon is not running (unknown, not "disconnected"). */
  connected: boolean | null;
}

/** The popup's one-line pointer to a wizard page that still has an open question. */
export interface SetupHint {
  id: string;
  page: string;
}

/** The wizard's optional checks (docs/specs/onboarding.md D11). */
export type SetupProbeKind = "popup" | "channel" | "agent";

/** How a check ended; `cancelled` is also what closing the test question without answering gives. */
export type SetupProbeOutcome = "ok" | "timeout" | "cancelled" | "failed";

/** The `setup-probe-result` event: a check ended. */
export interface SetupProbeResult {
  probeId: string;
  kind: SetupProbeKind;
  /** Which channel a `channel` check ran for. */
  channel?: string;
  outcome: SetupProbeOutcome;
  /** The Agent family whose question the `agent` check saw. */
  agentKind?: string;
}
