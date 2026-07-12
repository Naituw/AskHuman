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
}

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

export interface ConfirmChoice {
  id: string;
  label: string;
  description: string;
  role: ConfirmActionRole;
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

export type InteractionRequest =
  | { type: "ask"; request: AskRequest }
  | { type: "confirm"; request: ConfirmRequest };

export interface MessagePrompt {
  text: string;
  files: FileAttachment[];
}

/** 单个预定义选项：文本 + 是否为提问方（AI）的推荐答案。 */
export interface OptionItem {
  text: string;
  recommended: boolean;
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

export type ThemeMode = "system" | "light" | "dark";

export type PopupAnimation = "none" | "document" | "alert";

export type WindowEffect = "glass" | "blur";

export interface PopupInit {
  /** Current interaction. A prewarmed popup returns null until assigned. */
  interaction: InteractionRequest | null;
  theme: ThemeMode;
  alwaysOnTop: boolean;
  sourceName: string;
  /** 来源 workspace 完整路径（hover 显示）；空表示未知，前端隐藏该元素。 */
  project: string;
  /** workspace 目录名（标题区展示）。 */
  projectName: string;
  /** 发起本次提问的 agent 家族（claude/codex/cursor）；空表示未识别，不显示 agent badge。 */
  agentKind?: string | null;
  /** 发起本次提问的 agent 进程 pid；「聚焦终端」用。 */
  agentPid?: number | null;
  /** 界面语言原始值（auto/en/zh）；弹窗据此 applyLanguage，免再走 get_settings()。 */
  language?: string;
  /** 语音识别语言（BCP-47，如 zh-CN；auto 跟随系统）。 */
  speechLanguage?: string;
  /** 语音输入快捷键（规范串如 cmd+d；空串=关闭）。 */
  speechShortcut?: string;
  /** 实验：多问题弹窗纵向同时显示所有问题（默认关 = 旧版一次一题）。 */
  verticalQuestions?: boolean;
  /** 性能埋点是否开启（helper 收到 ASKHUMAN_PERF_ID）；前端据此决定是否上报 perf 标记。 */
  perf?: boolean;
  /** 性能测试：画完首帧后自动取消弹窗（仅 harness 用）。 */
  perfAutodismiss?: boolean;
  /** 方案6：本进程是否为预热弹窗（窗口起始隐藏）。为真时前端在内容绘制完成后调 `popup_show_window` 上屏。 */
  warm?: boolean;
  /** 提问创建时刻（epoch 毫秒）：弹窗据此显示相对时间（几秒/分钟/小时前），超过一天显示绝对时间。0=未知。 */
  createdAtMs?: number;
}

export interface QuestionAnswer {
  selectedOptions: string[];
  userInput: string;
  images: ImageAttachment[];
  files: string[];
}

export interface PopupSubmission {
  answers: QuestionAnswer[];
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
  /** Channel that submitted / cancelled: popup / dingding / feishu / telegram. */
  channel: string;
  action: ChannelAction;
  isMarkdown: boolean;
  message: MessagePrompt;
  questions: Question[];
  answers: HistoryAnswer[];
}

/** Aggregated project info for the history window's project picker. */
export interface ProjectInfo {
  key: string;
  name: string;
  count: number;
  lastMs: number;
}

/** History window init payload. */
export interface HistoryInit {
  theme: ThemeMode;
  /** 界面语言（已解析为 en/zh）；历史窗口据此 applyLanguage。 */
  lang: string;
  project: string;
  projectName: string;
}

/** Agent 状态窗口 init 负载（实验性功能）。 */
export interface AgentsInit {
  theme: ThemeMode;
  lang: string;
}

export type AgentKind = "claude" | "codex" | "cursor" | "grok";

/** 生命周期 hook 安装状态（实验区开关据此渲染）。 */
export interface LifecycleStatus {
  installed: boolean;
  outdated: boolean;
  supported: boolean;
}

export type AgentRunState = "working" | "idle" | "ended";

/** 单个被追踪 agent（一条 session）的快照记录。 */
export interface AgentRecord {
  /** 稳定数字编号（当前 daemon 生命周期内单调、不复用）；供 IM `/status <编号>` 寻址。 */
  seq?: number;
  kind: AgentKind;
  sessionId: string;
  pid?: number | null;
  title?: string | null;
  cwd?: string | null;
  startedAt: number;
  lastActivity: number;
  state: AgentRunState;
  endedAt?: number | null;
  /** 所在终端类型（apple-terminal/iterm2/vscode/…/other）；用于「聚焦终端」按钮显隐。 */
  terminal?: string | null;
  /** 实时「当前工具」（hook 上报，仅 snapshot、不落盘）：`{name, object?, at}`。GUI 暂不消费。 */
  currentTool?: { name: string; object?: string | null; at: number } | null;
  /** 有待送达的插话消息（daemon 注入；驱动「待送达」徽标与撤回按钮）。 */
  pendingInterject?: boolean;
}

/** 插话 composer 窗口 init 负载。 */
export interface InterjectInit {
  theme: ThemeMode;
  lang: string;
  /** 待送达全文（预填编辑；空 = 无待送达）。 */
  text: string;
  /** 待送达条数。 */
  entries: number;
}

export type UiLanguage = "auto" | "en" | "zh";

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
  /** 回复历史保留条数上限。默认 200；0 = 停止新增记录（但保留旧记录）。 */
  historyLimit: number;
  /** Built-in popup sound. Empty disables it; macOS stores a name, Linux uses a toggle. */
  popupSound: string;
  /** Menu bar / tray status icon mode (off/active/always). Desktop only (macOS/Linux). */
  menuBarIcon: MenuBarIconMode;
  /** Popup pre-warm (faster popups by keeping one mounted, hidden helper ready). Default true. */
  popupPrewarm: boolean;
  /** Daemon lifecycle: activity（按需起+空闲退出）/ keepalive（常驻+开机自启）。 */
  daemonLifecycle: DaemonLifecycleMode;
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
  width: number;
  height: number;
  rememberSize: boolean;
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

export interface ChannelsConfig {
  popup: PopupChannelConfig;
  telegram: TelegramChannelConfig;
  dingding: DingTalkChannelConfig;
  feishu: FeishuChannelConfig;
  slack: SlackChannelConfig;
  /** 「IM 渠道按需发送」开关（默认关；显式配置的用户设置保持原值）。 */
  autoActivation: boolean;
  /** 「自动结束 watch」——「按需发送」子开关（默认开，仅 autoActivation 开时生效）。 */
  autoEndWatch: boolean;
}

/** 实验性功能开关（默认隐藏；开启后显示「实验」Tab）。 */
export interface ExperimentalConfig {
  enabled: boolean;
  /** 多问题弹窗纵向同时显示所有问题（默认关 = 旧版一次一题）。 */
  verticalQuestions: boolean;
}

export interface AppConfig {
  general: GeneralConfig;
  channels: ChannelsConfig;
  experimental: ExperimentalConfig;
}

/** Whether each channel secret is currently stored (drives the "Saved" placeholder). */
export interface SecretsPresent {
  dingdingSecret: boolean;
  feishuSecret: boolean;
  telegramToken: boolean;
  slackBotToken: boolean;
  slackAppToken: boolean;
}

/** Settings payload: config with secrets blanked + per-secret presence flags. */
export interface SettingsPayload {
  config: AppConfig;
  secretsPresent: SecretsPresent;
}

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

export type AgentId = "cursor" | "claude" | "codex" | "grok";

export interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseNotes: string;
  sourceUrl: string;
  isNpm: boolean;
}

export interface PushedUpdateState {
  available: boolean;
  latestVersion: string;
  pending: boolean;
}

/** 调用方 agent 的异步解析结果（方案5/b）：daemon walk 出家族 + pid 后经 `agent-resolved` 后推弹窗。 */
export interface PushedAgent {
  kind?: string | null;
  pid?: number | null;
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
  permission: PermissionStatus;
  permissionNeedsUpdate: boolean;
  stop: StopStatus;
  mcpConfigPath: string;
  mcpConfigInstalled: boolean;
}

export interface StopStatus {
  supported: boolean;
  enabled: boolean;
  installed: boolean;
  outdated: boolean;
  otherHandlersDetected: boolean;
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
