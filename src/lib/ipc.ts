import { invoke } from "@tauri-apps/api/core";
import type {
  AgentsInit,
  AppConfig,
  ChannelIssue,
  DingTalkDetectArgs,
  DingTalkTestArgs,
  DingTalkWaitArgs,
  FeishuDetectArgs,
  FeishuTestArgs,
  FeishuWaitArgs,
  AgentId,
  AgentKind,
  AgentTaskReadiness,
  AgentTaskWorkspace,
  AgentMode,
  AgentModeStatus,
  ClaudeHookStatus,
  HistoryEntry,
  HistoryInit,
  HookStatus,
  InterjectInit,
  LifecycleStatus,
  PopupInit,
  PopupSoundSupport,
  PushedAgent,
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

/** 上报一个前端性能埋点（`stage` + 前端 epoch ms 时间戳）；埋点关闭时后端为 no-op。 */
export const perfMark = (stage: string, ts: number) =>
  invoke<void>("perf_mark", { stage, ts });

/** 异步解析指定 agent pid 所在终端类型（独立于 popup_init，避免进程链 ps 拖慢弹窗首屏）。 */
export const popupAgentTerminal = (pid: number) =>
  invoke<string | null>("popup_agent_terminal", { pid });

/** 拉取调用方 agent 的异步解析结果初值（方案5/b；之后靠 `agent-resolved` 事件实时更新）。 */
export const popupAgentResolved = () =>
  invoke<PushedAgent>("popup_agent_resolved");

/** 方案6：预热弹窗把本次请求内容绘制完成后调用，让后端把隐藏的窗口上屏（延后 show，杜绝闪现）。 */
export const popupShowWindow = () => invoke<void>("popup_show_window");

export const submitPopup = (submission: PopupSubmission) =>
  invoke<void>("submit_popup", { submission });

export const submitConfirmAction = (
  choiceIndex: number,
  comment?: string | null
) => invoke<void>("submit_confirm_action", { choiceIndex, comment });

export const confirmPopupReady = () => invoke<void>("confirm_popup_ready");

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

export const agentTaskWorkspaces = (refresh = false) =>
  invoke<AgentTaskWorkspace[]>("agent_task_workspaces", { refresh });
export const agentTaskWorkspaceAdd = (path: string) =>
  invoke<AgentTaskWorkspace>("agent_task_workspace_add", { path });
export const agentTaskWorkspacePick = () =>
  invoke<string | null>("agent_task_workspace_pick");
export const agentTaskWorkspacePin = (path: string, pinned: boolean) =>
  invoke<void>("agent_task_workspace_pin", { path, pinned });
export const agentTaskWorkspaceHide = (path: string, hidden: boolean) =>
  invoke<void>("agent_task_workspace_hide", { path, hidden });
export const agentTaskWorkspaceForget = (path: string) =>
  invoke<void>("agent_task_workspace_forget", { path });
export const agentTaskReadiness = () =>
  invoke<AgentTaskReadiness[]>("agent_task_readiness");
export const agentTaskTestTerminal = () =>
  invoke<void>("agent_task_test_terminal");

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

export const agentModeUpdateArtifact = (
  agent: AgentId,
  artifact: "rule" | "hook" | "mcp",
) => invoke<void>("agent_mode_update_artifact", { agent, artifact });

export const agentPermissionSet = (agent: AgentId, enabled: boolean) =>
  invoke<void>("agent_permission_set", { agent, enabled });

export const agentStopSet = (agent: AgentId, enabled: boolean) =>
  invoke<void>("agent_stop_set", { agent, enabled });

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

/** 聚焦某 Agent 所在终端（v1 仅 macOS / Terminal.app）。失败抛错由调用方静默处理。 */
export const focusAgentTerminal = (pid: number) =>
  invoke<void>("focus_agent_terminal", { pid });

/** 手动把某 agent 置为「空闲」（纠正漏 hook 卡「工作中」）。即发即走，daemon 改后推回新快照。 */
export const agentForceIdle = (sessionId: string) =>
  invoke<void>("agent_force_idle", { sessionId });

/** 打开某 agent 的插话 composer 窗口（经统一宿主路由，每 session 全局单窗）。 */
export const openInterject = (
  sessionId: string,
  kind: string | null,
  cwd: string | null,
) => invoke<void>("open_interject", { sessionId, kind, cwd });

/** 插话窗口初始化：登记 composer 打开 + 取待送达预填全文。 */
export const interjectInit = (sessionId: string) =>
  invoke<InterjectInit>("interject_init", { sessionId });

/** 提交插话（整体覆盖待送达队列；空文本＝清空），随后后端关连接、关窗口。 */
export const interjectSubmit = (sessionId: string, text: string) =>
  invoke<void>("interject_submit", { sessionId, text });

/** 取消插话（队列不动），后端关连接、关窗口。 */
export const interjectCancel = (sessionId: string) =>
  invoke<void>("interject_cancel", { sessionId });

/** 撤回某 session 的全部待送达插话。 */
export const interjectClear = (sessionId: string) =>
  invoke<void>("interject_clear", { sessionId });

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

/** 渠道健康快照（R7）：各渠道最近未恢复的故障；daemon 未运行返回空。 */
export const channelHealth = () =>
  invoke<ChannelIssue[]>("channel_health");

export const popupUpdateState = () =>
  invoke<PushedUpdateState>("popup_update_state");
