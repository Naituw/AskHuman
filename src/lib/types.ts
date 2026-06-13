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
  request: AskRequest;
  theme: ThemeMode;
  alwaysOnTop: boolean;
  sourceName: string;
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
  project: string;
  projectName: string;
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
}

export interface AppConfig {
  general: GeneralConfig;
  channels: ChannelsConfig;
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

export type AgentId = "cursor" | "claude" | "codex";

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

export interface RuleStatus {
  installed: boolean;
  outdated: boolean;
  path: string;
  supported: boolean;
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
