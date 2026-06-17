import { invoke } from "@tauri-apps/api/core";
import type {
  AgentsInit,
  AppConfig,
  DingTalkDetectArgs,
  DingTalkTestArgs,
  DingTalkWaitArgs,
  FeishuDetectArgs,
  FeishuTestArgs,
  FeishuWaitArgs,
  AgentId,
  AgentKind,
  AgentMode,
  AgentModeStatus,
  ClaudeHookStatus,
  HistoryEntry,
  HistoryInit,
  HookStatus,
  LifecycleStatus,
  PopupInit,
  PopupSoundSupport,
  PushedUpdateState,
  RuleStatus,
  PopupSubmission,
  ProjectInfo,
  SecretActions,
  SettingsPayload,
  SlackDetectArgs,
  SlackTestArgs,
  SlackWaitArgs,
  TelegramTestArgs,
  ThemeMode,
  UpdateInfo,
  WindowEffect,
} from "./types";

export const popupInit = () => invoke<PopupInit>("popup_init");

export const submitPopup = (submission: PopupSubmission) =>
  invoke<void>("submit_popup", { submission });

export const cancelPopup = () => invoke<void>("cancel_popup");

export const openPath = (path: string) => invoke<void>("open_path", { path });

export const previewAttachments = (paths: string[], index: number) =>
  invoke<void>("preview_attachments", { paths, index });

export const refocusPreview = () => invoke<void>("refocus_preview");

export const closePreview = () => invoke<void>("close_preview");

export const readImageDataUrl = (path: string) =>
  invoke<string>("read_image_data_url", { path });

export const fileIconDataUrl = (path: string) =>
  invoke<string>("file_icon_data_url", { path });

export const showAttachmentMenu = (path: string) =>
  invoke<void>("show_attachment_menu", { path });

export const getSettings = () => invoke<SettingsPayload>("get_settings");

export const saveSettings = (config: AppConfig, secretActions: SecretActions) =>
  invoke<void>("save_settings", { config, secretActions });

export const getPrompt = (variant?: "cli" | "mcp") =>
  invoke<string>("get_prompt", { variant });

export const openTestPopup = () => invoke<void>("open_test_popup");

export const popupSoundSupport = () =>
  invoke<PopupSoundSupport>("popup_sound_support");

export const playPopupSound = (name: string) =>
  invoke<void>("play_popup_sound", { name });

export const setTheme = (theme: ThemeMode) =>
  invoke<void>("set_theme", { theme });

export const updateTheme = (theme: ThemeMode) =>
  invoke<void>("update_theme", { theme });

export const openSettings = () => invoke<void>("open_settings");

export const openHistory = () => invoke<void>("open_history");

export const historyInit = () => invoke<HistoryInit>("history_init");

export const agentsInit = () => invoke<AgentsInit>("agents_init");

export const agentsStartSubscription = () =>
  invoke<void>("agents_start_subscription");

export const getHistory = (project: string | null, all: boolean) =>
  invoke<HistoryEntry[]>("get_history", { project, all });

export const getHistoryProjects = () =>
  invoke<ProjectInfo[]>("get_history_projects");

export const historyCount = () => invoke<number>("history_count");

export const trimHistory = (limit: number) =>
  invoke<number>("trim_history", { limit });

export const clearHistory = (all: boolean, project: string | null) =>
  invoke<void>("clear_history", { all, project });

export const applyWindowEffect = (effect: WindowEffect) =>
  invoke<void>("apply_window_effect", { effect });

export const startSpeech = (locale: string) =>
  invoke<void>("start_speech", { locale });

export const stopSpeech = () => invoke<void>("stop_speech");

export const flushSpeech = () => invoke<void>("flush_speech");

export const speechAvailable = () => invoke<boolean>("speech_available");

export const cursorHookStatus = () => invoke<HookStatus>("cursor_hook_status");

export const cursorHookInstall = () => invoke<string>("cursor_hook_install");

export const cursorHookUpdate = () => invoke<string>("cursor_hook_update");

export const cursorHookUninstall = () => invoke<string>("cursor_hook_uninstall");

export const cursorHookReveal = () => invoke<void>("cursor_hook_reveal");

export const claudeHookStatus = () =>
  invoke<ClaudeHookStatus>("claude_hook_status");

export const claudeHookInstall = () => invoke<string>("claude_hook_install");

export const claudeHookUpdate = () => invoke<string>("claude_hook_update");

export const claudeHookUninstall = () =>
  invoke<string>("claude_hook_uninstall");

export const claudeHookReveal = () => invoke<void>("claude_hook_reveal");

export const agentRuleStatus = (agent: AgentId) =>
  invoke<RuleStatus>("agent_rule_status", { agent });

export const agentRuleInstall = (agent: AgentId) =>
  invoke<string>("agent_rule_install", { agent });

export const agentRuleUpdate = (agent: AgentId) =>
  invoke<string>("agent_rule_update", { agent });

export const agentRuleUninstall = (agent: AgentId) =>
  invoke<string>("agent_rule_uninstall", { agent });

export const agentRuleReveal = (agent: AgentId) =>
  invoke<void>("agent_rule_reveal", { agent });

export const agentRuleOpen = (agent: AgentId) =>
  invoke<void>("agent_rule_open", { agent });

export const agentModeStatus = (agent: AgentId) =>
  invoke<AgentModeStatus>("agent_mode_status", { agent });

export const agentModeSet = (agent: AgentId, mode: AgentMode) =>
  invoke<void>("agent_mode_set", { agent, mode });

export const agentModeUpdate = (agent: AgentId) =>
  invoke<void>("agent_mode_update", { agent });

export const mcpConfigReveal = (agent: AgentId) =>
  invoke<void>("mcp_config_reveal", { agent });

export const mcpConfigOpen = (agent: AgentId) =>
  invoke<void>("mcp_config_open", { agent });

export const mcpCommandPath = () => invoke<string>("mcp_command_path");

export const agentHookReveal = (agent: AgentId) =>
  invoke<void>("agent_hook_reveal", { agent });

export const agentHookOpen = (agent: AgentId) =>
  invoke<void>("agent_hook_open", { agent });

export const agentLifecycleStatus = (agent: AgentKind) =>
  invoke<LifecycleStatus>("agent_lifecycle_status", { agent });

export const agentLifecycleInstall = (agent: AgentKind) =>
  invoke<string>("agent_lifecycle_install", { agent });

export const agentLifecycleUninstall = (agent: AgentKind) =>
  invoke<string>("agent_lifecycle_uninstall", { agent });

export const telegramTest = (args: TelegramTestArgs) =>
  invoke<string>("telegram_test", { args });

export const dingtalkTest = (args: DingTalkTestArgs) =>
  invoke<string>("dingtalk_test", { args });

export const dingtalkDetectPrepare = (args: DingTalkDetectArgs) =>
  invoke<string>("dingtalk_detect_prepare", { args });

export const dingtalkDetectWait = (args: DingTalkWaitArgs) =>
  invoke<string>("dingtalk_detect_wait", { args });

export const feishuTest = (args: FeishuTestArgs) =>
  invoke<string>("feishu_test", { args });

export const feishuDetectPrepare = (args: FeishuDetectArgs) =>
  invoke<string>("feishu_detect_prepare", { args });

export const feishuDetectWait = (args: FeishuWaitArgs) =>
  invoke<string>("feishu_detect_wait", { args });

export const slackTest = (args: SlackTestArgs) =>
  invoke<string>("slack_test", { args });

export const slackDetectPrepare = (args: SlackDetectArgs) =>
  invoke<string>("slack_detect_prepare", { args });

export const slackDetectWait = (args: SlackWaitArgs) =>
  invoke<string>("slack_detect_wait", { args });

// 取消正在进行的「自动识别」等待（三家共用）。
export const detectCancel = () => invoke<void>("detect_cancel");

// ===== 版本自更新 =====

export const getAppVersion = () => invoke<string>("get_app_version");

export const updateCheck = (manual: boolean) =>
  invoke<UpdateInfo>("update_check", { manual });

export const updateGetNotes = (aggregate: boolean) =>
  invoke<string>("update_get_notes", { aggregate });

export const updateGetVersionNotes = (version: string) =>
  invoke<string>("update_get_version_notes", { version });

export const updateApply = () => invoke<void>("update_apply");

export const updateDismiss = (version: string) =>
  invoke<void>("update_dismiss", { version });

export const restartSettings = () => invoke<void>("restart_settings");

export const popupUpdateState = () =>
  invoke<PushedUpdateState>("popup_update_state");
