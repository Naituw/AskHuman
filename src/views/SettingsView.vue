<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyLanguage } from "../i18n";
import {
  agentModeStatus,
  agentModeSet,
  agentModeUpdate,
  agentRuleReveal,
  agentRuleOpen,
  mcpConfigReveal,
  mcpConfigOpen,
  mcpCommandPath,
  agentHookReveal,
  agentHookOpen,
  agentLifecycleStatus,
  agentLifecycleInstall,
  agentLifecycleUninstall,
  applyWindowEffect,
  dingtalkDetectPrepare,
  dingtalkDetectWait,
  dingtalkTest,
  feishuDetectPrepare,
  feishuDetectWait,
  feishuTest,
  getAppVersion,
  getPrompt,
  getSettings,
  historyCount,
  openPath,
  openTestPopup,
  playPopupSound,
  popupSoundSupport,
  restartSettings,
  detectCancel,
  saveSettings,
  setTheme,
  slackDetectPrepare,
  slackDetectWait,
  slackTest,
  telegramTest,
  trimHistory,
  updateApply,
  updateCheck,
  updateGetNotes,
  updateGetVersionNotes,
} from "../lib/ipc";
import { applyTheme } from "../lib/theme";
import { renderMarkdown } from "../lib/markdown";
import {
  eventToSpec,
  formatShortcut,
  isModifierOnly,
  shortcutConflict,
  specToString,
  type ConflictReason,
} from "../lib/shortcut";
import { isGlassSupported } from "tauri-plugin-liquid-glass-api";
import type {
  AgentId,
  AgentKind,
  AgentMode,
  AgentModeStatus,
  AppConfig,
  LifecycleStatus,
  MenuBarIconMode,
  PopupAnimation,
  PopupSoundSupport,
  SecretAction,
  SecretActions,
  SecretsPresent,
  ThemeMode,
  UiLanguage,
  UpdateInfo,
  WindowEffect,
} from "../lib/types";

const { t } = useI18n();

// 出现动画为 macOS 原生窗口能力，其它平台不展示选择器。
const isMac = navigator.userAgent.toLowerCase().includes("mac");
const isWindows = navigator.userAgent.toLowerCase().includes("win");

// 「在文件管理器中显示」的按平台措辞（访达 / 文件资源管理器 / 文件管理器），单一来源。
const revealLabel = computed(() => {
  if (isMac) return t("settings.integration.revealInFinder");
  if (isWindows) return t("settings.integration.revealInExplorer");
  return t("settings.integration.revealInFileManager");
});

type Tab = "general" | "integration" | "channel" | "experimental";

const config = ref<AppConfig | null>(null);
const activeTab = ref<Tab>("general");

// Secrets are never loaded into the UI; we only know whether each is configured (for the
// placeholder) and track an explicit "cleared" intent until the next save.
const secretsPresent = ref<SecretsPresent>({
  dingdingSecret: false,
  feishuSecret: false,
  telegramToken: false,
  slackBotToken: false,
  slackAppToken: false,
});
const secretCleared = ref({
  dingding: false,
  feishu: false,
  telegram: false,
  slackBot: false,
  slackApp: false,
});
const SECRET_PLACEHOLDER = "••••••••";

// tab 按钮同时是窗口拖拽区：用屏幕坐标区分「点击切换」与「拖动移窗」。
// 原生拖窗时窗口跟随光标，clientX/Y 几乎不变，故必须用 screenX/Y。
const tabDown = ref<{ x: number; y: number } | null>(null);
function onTabDown(e: MouseEvent) {
  tabDown.value = { x: e.screenX, y: e.screenY };
}
function onTabClick(tab: Tab, e: MouseEvent) {
  const d = tabDown.value;
  tabDown.value = null;
  if (d && Math.hypot(e.screenX - d.x, e.screenY - d.y) > 4) return;
  activeTab.value = tab;
}
const prompt = ref("");
const promptCopied = ref(false);
// 手动集成卡的提示词变体：CLI 版 / MCP 版（切换即重载正文）。
const promptVariant = ref<"cli" | "mcp">("cli");

// 各 Agent 的展示信息。Codex 没有「超时 Hook」概念（hasTimeoutHook=false），
// 且无法延长 CLI 超时，故推荐 MCP；Cursor / Claude Code 有可靠超时 Hook，推荐 CLI。
const AGENTS: {
  id: AgentId;
  title: string;
  hasTimeoutHook: boolean;
  recommended: AgentMode;
}[] = [
  { id: "cursor", title: "Cursor", hasTimeoutHook: true, recommended: "cli" },
  { id: "claude", title: "Claude Code", hasTimeoutHook: true, recommended: "cli" },
  { id: "codex", title: "Codex", hasTimeoutHook: false, recommended: "mcp" },
];

const emptyMode = (): AgentModeStatus => ({
  mode: "none",
  needsUpdate: false,
  rulePath: "",
  ruleInstalled: false,
  timeoutHookSupported: false,
  timeoutHookInstalled: false,
  mcpConfigPath: "",
  mcpConfigInstalled: false,
});
const modes = ref<Record<AgentId, AgentModeStatus>>({
  cursor: emptyMode(),
  claude: emptyMode(),
  codex: emptyMode(),
});
const modeBusy = ref<Record<AgentId, boolean>>({
  cursor: false,
  claude: false,
  codex: false,
});
const modeMessage = ref<Record<AgentId, string | null>>({
  cursor: null,
  claude: null,
  codex: null,
});
const modeError = ref<Record<AgentId, boolean>>({
  cursor: false,
  claude: false,
  codex: false,
});

async function refreshMode(agent: AgentId) {
  modes.value[agent] = await agentModeStatus(agent);
}

// 一键切换到目标模式（含「未集成」）：自动卸旧装新。
async function setMode(agent: AgentId, mode: AgentMode) {
  if (modeBusy.value[agent] || modes.value[agent].mode === mode) return;
  modeBusy.value[agent] = true;
  modeMessage.value[agent] = null;
  try {
    await agentModeSet(agent, mode);
    modeError.value[agent] = false;
  } catch (e) {
    modeMessage.value[agent] = String(e);
    modeError.value[agent] = true;
  } finally {
    modeBusy.value[agent] = false;
    await refreshMode(agent);
  }
}

// 把当前模式的全部产物刷新到最新（不切换模式）。
async function updateMode(agent: AgentId) {
  modeBusy.value[agent] = true;
  modeMessage.value[agent] = null;
  try {
    await agentModeUpdate(agent);
    modeError.value[agent] = false;
  } catch (e) {
    modeMessage.value[agent] = String(e);
    modeError.value[agent] = true;
  } finally {
    modeBusy.value[agent] = false;
    await refreshMode(agent);
  }
}

// 「打开」下拉菜单：当前展开菜单的 key（`${agent}:${kind}`，null = 全部收起）。
type FileKind = "rule" | "hook" | "mcp";
const openMenuKey = ref<string | null>(null);
function toggleOpenMenu(key: string) {
  openMenuKey.value = openMenuKey.value === key ? null : key;
}
function closeOpenMenu() {
  openMenuKey.value = null;
}
function revealFile(agent: AgentId, kind: FileKind) {
  if (kind === "mcp") mcpConfigReveal(agent);
  else if (kind === "hook") agentHookReveal(agent);
  else agentRuleReveal(agent);
  closeOpenMenu();
}
function openFile(agent: AgentId, kind: FileKind) {
  if (kind === "mcp") mcpConfigOpen(agent);
  else if (kind === "hook") agentHookOpen(agent);
  else agentRuleOpen(agent);
  closeOpenMenu();
}

async function loadPrompt() {
  prompt.value = await getPrompt(promptVariant.value);
}

function setPromptVariant(v: "cli" | "mcp") {
  if (promptVariant.value === v) return;
  promptVariant.value = v;
  void loadPrompt();
}

// MCP 手动配置示例。直接填入当前可执行文件绝对路径（与自动集成写入一致），免用户手改；
// 取不到时退回占位符。
const MCP_EXE_PLACEHOLDER = "<absolute path to AskHuman>";
const mcpExePath = ref(MCP_EXE_PLACEHOLDER);
const mcpExampleJson = computed(
  () => `{
  "mcpServers": {
    "askhuman": {
      "command": "${mcpExePath.value}",
      "args": ["mcp"],
      "timeout": 86400000
    }
  }
}`,
);
const mcpExampleToml = computed(
  () => `[mcp_servers.askhuman]
command = "${mcpExePath.value}"
args = ["mcp"]
startup_timeout_sec = 30
tool_timeout_sec = 86400`,
);
const mcpJsonCopied = ref(false);
const mcpTomlCopied = ref(false);

async function copyMcpExample(kind: "json" | "toml") {
  const text = kind === "json" ? mcpExampleJson.value : mcpExampleToml.value;
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    /* 忽略：剪贴板不可用时静默 */
  }
  const flag = kind === "json" ? mcpJsonCopied : mcpTomlCopied;
  flag.value = true;
  setTimeout(() => (flag.value = false), 1500);
}

const telegramTesting = ref(false);
const telegramMessage = ref<string | null>(null);
const telegramError = ref(false);

// 「自动识别」取消标记（三家共用）：点了取消后，等待会被中止并 reject，catch 据此走中性收尾而非报错。
const detectCancelled = ref(false);

// 取消正在进行的自动识别（三家共用）：置标记 + 通知后端中止等待（连带释放临时长连接）。
async function cancelDetect() {
  detectCancelled.value = true;
  try {
    await detectCancel();
  } catch {
    /* 取消本身失败可忽略：等待仍会按既有路径超时收尾 */
  }
}

const dingtalkTesting = ref(false);
const dingtalkDetecting = ref(false);
const dingtalkDetectCode = ref<string | null>(null);
const dingtalkMessage = ref<string | null>(null);
const dingtalkError = ref(false);

const feishuTesting = ref(false);
const feishuDetecting = ref(false);
const feishuDetectCode = ref<string | null>(null);
const feishuMessage = ref<string | null>(null);
const feishuError = ref(false);

const slackTesting = ref(false);
const slackDetecting = ref(false);
const slackDetectCode = ref<string | null>(null);
const slackMessage = ref<string | null>(null);
const slackError = ref(false);

function clamp(v: number, min: number, max: number) {
  return Math.min(max, Math.max(min, v));
}

// 是否支持 Liquid Glass（macOS 26+）：决定「玻璃/模糊」开关是否显示。
const glassSupported = ref(true);

// Build a secret's edit intent: a typed value wins (set); else an explicit clear; else unchanged.
function secretActionFor(value: string, cleared: boolean): SecretAction {
  if (value && value.length > 0) return { kind: "set", value };
  if (cleared) return { kind: "clear" };
  return { kind: "unchanged" };
}

async function persist() {
  if (!config.value) return;
  const c = config.value.channels;
  const actions: SecretActions = {
    dingdingSecret: secretActionFor(
      c.dingding.clientSecret,
      secretCleared.value.dingding
    ),
    feishuSecret: secretActionFor(c.feishu.appSecret, secretCleared.value.feishu),
    telegramToken: secretActionFor(
      c.telegram.botToken,
      secretCleared.value.telegram
    ),
    slackBotToken: secretActionFor(
      c.slack.botToken,
      secretCleared.value.slackBot
    ),
    slackAppToken: secretActionFor(
      c.slack.appToken,
      secretCleared.value.slackApp
    ),
  };
  await saveSettings(config.value, actions);
  // Reflect the saved state: a set secret becomes a "Saved" placeholder, a cleared one becomes
  // empty. Wipe the field so the secret is never re-sent on subsequent saves.
  finalizeSecret(actions.dingdingSecret, "dingdingSecret", "dingding");
  finalizeSecret(actions.feishuSecret, "feishuSecret", "feishu");
  finalizeSecret(actions.telegramToken, "telegramToken", "telegram");
  finalizeSecret(actions.slackBotToken, "slackBotToken", "slackBot");
  finalizeSecret(actions.slackAppToken, "slackAppToken", "slackApp");
}

type ClearedKey = "dingding" | "feishu" | "telegram" | "slackBot" | "slackApp";

function finalizeSecret(
  action: SecretAction,
  presentKey: keyof SecretsPresent,
  clearedKey: ClearedKey
) {
  if (!config.value) return;
  if (action.kind === "set") secretsPresent.value[presentKey] = true;
  else if (action.kind === "clear") secretsPresent.value[presentKey] = false;
  if (action.kind !== "unchanged") {
    const c = config.value.channels;
    if (clearedKey === "dingding") c.dingding.clientSecret = "";
    else if (clearedKey === "feishu") c.feishu.appSecret = "";
    else if (clearedKey === "slackBot") c.slack.botToken = "";
    else if (clearedKey === "slackApp") c.slack.appToken = "";
    else c.telegram.botToken = "";
  }
  secretCleared.value[clearedKey] = false;
}

// "Clear" button: drop the saved secret (deletes the keychain entry on save) and re-persist so the
// daemon reloads with the secret gone.
function clearSecret(channel: ClearedKey) {
  if (!config.value) return;
  const c = config.value.channels;
  if (channel === "dingding") c.dingding.clientSecret = "";
  else if (channel === "feishu") c.feishu.appSecret = "";
  else if (channel === "slackBot") c.slack.botToken = "";
  else if (channel === "slackApp") c.slack.appToken = "";
  else c.telegram.botToken = "";
  secretCleared.value[channel] = true;
  persist();
}

async function changeTheme(theme: ThemeMode) {
  if (!config.value) return;
  config.value.general.theme = theme;
  applyTheme(theme);
  await setTheme(theme);
  await persist();
}

// 切换界面语言：本窗口立即生效；persist 广播 settings-updated 令其它窗口同步。
async function changeLanguage(lang: UiLanguage) {
  if (!config.value) return;
  config.value.general.language = lang;
  applyLanguage(lang);
  await persist();
}

// 其它窗口改了语言时，本窗口也同步切换。
let unlistenSettings: UnlistenFn | null = null;
onBeforeUnmount(() => unlistenSettings?.());

async function changeAnimation(anim: PopupAnimation) {
  if (!config.value) return;
  config.value.general.appearAnimation = anim;
  await persist();
}

// 菜单栏图标三态（off/active/always）。仅持久化；宿主进程监听 config 变化后自行
// 建/移图标、装/卸登录项（见 src-tauri app::gui_host）。
async function changeMenuBarIcon(mode: MenuBarIconMode) {
  if (!config.value) return;
  config.value.general.menuBarIcon = mode;
  await persist();
}

// Popup sound support: named choices on macOS, toggle on Linux, hidden otherwise.
const soundSupport = ref<PopupSoundSupport>({ kind: "none", names: [] });

async function changePopupSound(value: string) {
  if (!config.value) return;
  config.value.general.popupSound = value;
  await persist();
  // Preview immediately after selecting a non-empty sound.
  if (value) playPopupSound(value).catch(() => {});
}

function previewSound() {
  const name = config.value?.general.popupSound;
  if (name) playPopupSound(name).catch(() => {});
}

// 当前历史总条数（用于「超额」提示与「立即清理」）。
const historyTotal = ref(0);
const overLimit = computed(() => {
  const limit = config.value?.general.historyLimit ?? 0;
  return historyTotal.value > limit;
});

// 改保留条数：仅持久化；裁剪发生在下次 AskHuman 或点击「立即清理」。
async function changeHistoryLimit(raw: number) {
  if (!config.value) return;
  const v = Number.isFinite(raw) ? Math.max(0, Math.floor(raw)) : 0;
  config.value.general.historyLimit = v;
  await persist();
}

async function cleanHistoryNow() {
  const limit = config.value?.general.historyLimit ?? 0;
  historyTotal.value = await trimHistory(limit);
}

// 语音识别语言下拉项：第一项「跟随系统」(auto) + 常用语言（BCP-47）。
const SPEECH_LANGUAGES: { value: string; label: string }[] = [
  // auto 的显示文案在模板里走 i18n（settings.speech.languageSystem）。
  { value: "auto", label: "" },
  { value: "zh-CN", label: "简体中文" },
  { value: "zh-TW", label: "繁体中文" },
  { value: "en-US", label: "English (US)" },
  { value: "ja-JP", label: "日本語" },
  { value: "ko-KR", label: "한국어" },
];

async function changeSpeechLanguage(lang: string) {
  if (!config.value) return;
  config.value.general.speechLanguage = lang;
  await persist();
}

// 语音快捷键录制：点一下进入录制，直接按组合键录入；Esc 取消。
const recordingShortcut = ref(false);
const shortcutPreview = ref("");
const shortcutError = ref<ConflictReason | null>(null);
let shortcutHandler: ((e: KeyboardEvent) => void) | null = null;

function previewModifiers(e: KeyboardEvent): string {
  let o = "";
  if (e.ctrlKey) o += "⌃";
  if (e.altKey) o += "⌥";
  if (e.shiftKey) o += "⇧";
  if (e.metaKey) o += "⌘";
  return o ? o + "…" : "";
}

function stopRecordShortcut() {
  recordingShortcut.value = false;
  shortcutPreview.value = "";
  if (shortcutHandler) {
    window.removeEventListener("keydown", shortcutHandler, true);
    shortcutHandler = null;
  }
}

function startRecordShortcut() {
  if (recordingShortcut.value) return;
  recordingShortcut.value = true;
  shortcutError.value = null;
  shortcutPreview.value = "";
  shortcutHandler = (e: KeyboardEvent) => {
    // 捕获阶段拦截，避免触发浏览器/窗口默认行为（如 ⌘W 关窗）。
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      stopRecordShortcut();
      return;
    }
    if (isModifierOnly(e)) {
      shortcutPreview.value = previewModifiers(e);
      return;
    }
    const spec = eventToSpec(e);
    if (!spec) return;
    const reason = shortcutConflict(spec);
    if (reason) {
      shortcutError.value = reason;
      return;
    }
    if (config.value) {
      config.value.general.speechShortcut = specToString(spec);
      persist();
    }
    stopRecordShortcut();
  };
  window.addEventListener("keydown", shortcutHandler, true);
}

function clearShortcut() {
  if (!config.value) return;
  config.value.general.speechShortcut = "";
  shortcutError.value = null;
  persist();
  stopRecordShortcut();
}

onBeforeUnmount(stopRecordShortcut);

// 切换弹窗背景效果（仅 macOS 26+ 显示）。持久化后实时作用于已打开的 popup + 设置窗口。
async function changeWindowEffect(effect: WindowEffect) {
  if (!config.value) return;
  config.value.general.windowEffect = effect;
  await persist();
  try {
    // 后端同时切换 popup 与 settings 两个窗口（玻璃用插件、模糊用 set_effects）。
    await applyWindowEffect(effect);
  } catch (e) {
    console.error("切换窗口效果失败", e);
  }
}

function stepWidth(delta: number) {
  if (!config.value) return;
  config.value.channels.popup.width = clamp(
    config.value.channels.popup.width + delta,
    360,
    1200
  );
  persist();
}

function stepHeight(delta: number) {
  if (!config.value) return;
  config.value.channels.popup.height = clamp(
    config.value.channels.popup.height + delta,
    360,
    1400
  );
  persist();
}

// ===== 实验区：Agent 生命周期追踪开关 =====
const LIFECYCLE_KINDS: AgentKind[] = ["claude", "codex", "cursor"];

const lifecycleStatus = ref<Record<AgentKind, LifecycleStatus>>({
  claude: { installed: false, outdated: false, supported: true },
  codex: { installed: false, outdated: false, supported: true },
  cursor: { installed: false, outdated: false, supported: true },
});
const lifecycleBusy = ref<Record<AgentKind, boolean>>({
  claude: false,
  codex: false,
  cursor: false,
});
const lifecycleError = ref<Record<AgentKind, string | null>>({
  claude: null,
  codex: null,
  cursor: null,
});

async function refreshLifecycle() {
  for (const kind of LIFECYCLE_KINDS) {
    try {
      lifecycleStatus.value[kind] = await agentLifecycleStatus(kind);
    } catch (e) {
      lifecycleError.value[kind] = String(e);
    }
  }
}

// 开关切换：开 = 安装，关 = 卸载。失败时回滚显示并展示错误。
async function toggleLifecycle(kind: AgentKind, on: boolean) {
  if (lifecycleBusy.value[kind]) return;
  lifecycleBusy.value[kind] = true;
  lifecycleError.value[kind] = null;
  try {
    if (on) await agentLifecycleInstall(kind);
    else await agentLifecycleUninstall(kind);
    lifecycleStatus.value[kind] = await agentLifecycleStatus(kind);
  } catch (e) {
    lifecycleError.value[kind] = String(e);
    // 回滚到后端真实状态，避免开关与实际不一致。
    try {
      lifecycleStatus.value[kind] = await agentLifecycleStatus(kind);
    } catch {
      /* 状态查询也失败时保留现状 */
    }
  } finally {
    lifecycleBusy.value[kind] = false;
  }
}

// 打开「实验」开关时显露实验 Tab；关闭时若停留在实验 Tab 则退回通用。
async function toggleExperimental() {
  if (!config.value) return;
  if (!config.value.experimental.enabled && activeTab.value === "experimental") {
    activeTab.value = "general";
  }
  await persist();
  if (config.value.experimental.enabled) await refreshLifecycle();
}

function lifecycleLabel(kind: AgentKind): string {
  return t(`settings.experimental.${kind}`);
}

async function copyPrompt() {
  try {
    await navigator.clipboard.writeText(prompt.value);
  } catch {
    /* 忽略：剪贴板不可用时静默 */
  }
  promptCopied.value = true;
  setTimeout(() => (promptCopied.value = false), 1500);
}

async function runTelegramTest() {
  if (!config.value) return;
  telegramTesting.value = true;
  telegramMessage.value = null;
  const tg = config.value.channels.telegram;
  try {
    telegramMessage.value = await telegramTest({
      botToken: tg.botToken,
      chatId: tg.chatId,
      apiBaseUrl: tg.apiBaseUrl,
    });
    telegramError.value = false;
  } catch (e) {
    telegramMessage.value = String(e);
    telegramError.value = true;
  } finally {
    telegramTesting.value = false;
  }
}

async function runDingtalkTest() {
  if (!config.value) return;
  dingtalkTesting.value = true;
  dingtalkMessage.value = null;
  const dd = config.value.channels.dingding;
  try {
    dingtalkMessage.value = await dingtalkTest({
      clientId: dd.clientId,
      clientSecret: dd.clientSecret,
      userId: dd.userId,
    });
    dingtalkError.value = false;
  } catch (e) {
    dingtalkMessage.value = String(e);
    dingtalkError.value = true;
  } finally {
    dingtalkTesting.value = false;
  }
}

// 自动识别：先校验并取识别码 → 展示提示 → 等用户私聊发送该码 → 回填 userId。
async function runDingtalkDetect() {
  if (!config.value) return;
  const dd = config.value.channels.dingding;
  dingtalkDetecting.value = true;
  detectCancelled.value = false;
  dingtalkMessage.value = null;
  dingtalkDetectCode.value = null;
  try {
    const code = await dingtalkDetectPrepare({
      clientId: dd.clientId,
      clientSecret: dd.clientSecret,
    });
    dingtalkDetectCode.value = code;
    const userId = await dingtalkDetectWait({
      clientId: dd.clientId,
      clientSecret: dd.clientSecret,
      code,
    });
    dd.userId = userId;
    await persist();
    dingtalkError.value = false;
    dingtalkMessage.value = t("settings.channels.detected", { userId });
  } catch (e) {
    if (detectCancelled.value) {
      dingtalkMessage.value = null;
      dingtalkError.value = false;
    } else {
      dingtalkMessage.value = String(e);
      dingtalkError.value = true;
    }
  } finally {
    dingtalkDetecting.value = false;
    dingtalkDetectCode.value = null;
    detectCancelled.value = false;
  }
}

async function runFeishuTest() {
  if (!config.value) return;
  feishuTesting.value = true;
  feishuMessage.value = null;
  const fs = config.value.channels.feishu;
  try {
    feishuMessage.value = await feishuTest({
      appId: fs.appId,
      appSecret: fs.appSecret,
      openId: fs.openId,
      baseUrl: fs.baseUrl,
    });
    feishuError.value = false;
  } catch (e) {
    feishuMessage.value = String(e);
    feishuError.value = true;
  } finally {
    feishuTesting.value = false;
  }
}

// 自动识别：先校验并取识别码 → 展示提示 → 等用户私聊发送该码 → 回填 openId。
async function runFeishuDetect() {
  if (!config.value) return;
  const fs = config.value.channels.feishu;
  feishuDetecting.value = true;
  detectCancelled.value = false;
  feishuMessage.value = null;
  feishuDetectCode.value = null;
  try {
    const code = await feishuDetectPrepare({
      appId: fs.appId,
      appSecret: fs.appSecret,
      baseUrl: fs.baseUrl,
    });
    feishuDetectCode.value = code;
    const openId = await feishuDetectWait({
      appId: fs.appId,
      appSecret: fs.appSecret,
      baseUrl: fs.baseUrl,
      code,
    });
    fs.openId = openId;
    await persist();
    feishuError.value = false;
    feishuMessage.value = t("settings.channels.feishuDetected", { openId });
  } catch (e) {
    if (detectCancelled.value) {
      feishuMessage.value = null;
      feishuError.value = false;
    } else {
      feishuMessage.value = String(e);
      feishuError.value = true;
    }
  } finally {
    feishuDetecting.value = false;
    feishuDetectCode.value = null;
    detectCancelled.value = false;
  }
}

async function runSlackTest() {
  if (!config.value) return;
  slackTesting.value = true;
  slackMessage.value = null;
  const sl = config.value.channels.slack;
  try {
    slackMessage.value = await slackTest({
      botToken: sl.botToken,
      appToken: sl.appToken,
      userId: sl.userId,
    });
    slackError.value = false;
  } catch (e) {
    slackMessage.value = String(e);
    slackError.value = true;
  } finally {
    slackTesting.value = false;
  }
}

// 自动识别：先校验并取识别码 → 展示提示 → 等用户私聊发送该码 → 回填 userId。
async function runSlackDetect() {
  if (!config.value) return;
  const sl = config.value.channels.slack;
  slackDetecting.value = true;
  detectCancelled.value = false;
  slackMessage.value = null;
  slackDetectCode.value = null;
  try {
    const code = await slackDetectPrepare({
      botToken: sl.botToken,
      appToken: sl.appToken,
    });
    slackDetectCode.value = code;
    const userId = await slackDetectWait({
      botToken: sl.botToken,
      appToken: sl.appToken,
      code,
    });
    sl.userId = userId;
    await persist();
    slackError.value = false;
    slackMessage.value = t("settings.channels.slackDetected", { userId });
  } catch (e) {
    if (detectCancelled.value) {
      slackMessage.value = null;
      slackError.value = false;
    } else {
      slackMessage.value = String(e);
      slackError.value = true;
    }
  } finally {
    slackDetecting.value = false;
    slackDetectCode.value = null;
    detectCancelled.value = false;
  }
}

onMounted(async () => {
  const payload = await getSettings();
  config.value = payload.config;
  secretsPresent.value = payload.secretsPresent;
  applyTheme(config.value.general.theme);
  applyLanguage(config.value.general.language);
  unlistenSettings = await listen<{ language?: UiLanguage }>(
    "settings-updated",
    (e) => {
      if (e.payload.language) applyLanguage(e.payload.language);
    }
  );
  await loadPrompt();
  try {
    mcpExePath.value = await mcpCommandPath();
  } catch {
    /* 取不到路径时保留占位符 */
  }
  historyTotal.value = await historyCount();
  try {
    soundSupport.value = await popupSoundSupport();
  } catch {
    soundSupport.value = { kind: "none", names: [] };
  }
  await Promise.all(AGENTS.map((a) => refreshMode(a.id)));
  if (!isWindows && config.value.experimental.enabled) await refreshLifecycle();
  if (isMac) {
    try {
      glassSupported.value = await isGlassSupported();
    } catch {
      glassSupported.value = false;
    }
  }
  // 关于区：取本地版本，并静默检查一次（best-effort，失败不打扰）。
  try {
    appVersion.value = await getAppVersion();
  } catch {
    appVersion.value = "";
  }
  void checkUpdate(false);
});

// ===== 关于 / 版本自更新 =====
const appVersion = ref("");
const updateInfo = ref<UpdateInfo | null>(null);
const updateChecking = ref(false);
const updateApplying = ref(false);
const updateDone = ref(false);
const updateError = ref("");
const updateProgress = ref(0);
const notesHtml = ref("");
const releasesUrl = "https://github.com/Naituw/AskHuman/releases";

// 当前版本更新日志（折叠，懒加载；与「发现新版」的日志独立）。
const currentNotesOpen = ref(false);
const currentNotesHtml = ref("");
const currentNotesLoading = ref(false);
const currentNotesError = ref("");
const currentNotesLoaded = ref(false);

// 把后端错误转可读文案：限流（403/429，后端带 rate-limited 标记）→ 友好提示并引导手动下载 / 设 token；
// 其余沿用「<前缀>: <原始错误>」。
function updateErrText(e: unknown, prefixKey: string): string {
  const s = String(e);
  if (/rate-limited|\b403\b|\b429\b/i.test(s)) {
    return t("settings.about.rateLimited");
  }
  return `${t(`settings.about.${prefixKey}`)}: ${s}`;
}

async function toggleCurrentNotes() {
  currentNotesOpen.value = !currentNotesOpen.value;
  if (!currentNotesOpen.value || currentNotesLoaded.value || !appVersion.value) {
    return;
  }
  currentNotesLoading.value = true;
  currentNotesError.value = "";
  try {
    const notes = await updateGetVersionNotes(appVersion.value);
    currentNotesHtml.value = notes.trim() ? renderMarkdown(notes) : "";
    currentNotesLoaded.value = true;
  } catch (e) {
    currentNotesError.value = updateErrText(e, "notesFailed");
  } finally {
    currentNotesLoading.value = false;
  }
}

async function checkUpdate(manual: boolean) {
  if (updateChecking.value) return;
  updateChecking.value = true;
  updateError.value = "";
  try {
    const info = await updateCheck(manual);
    updateInfo.value = info;
    notesHtml.value = "";
    if (info.available) {
      try {
        const notes = await updateGetNotes(true);
        notesHtml.value = notes.trim() ? renderMarkdown(notes) : "";
      } catch {
        notesHtml.value = "";
      }
    }
  } catch (e) {
    updateError.value = updateErrText(e, "checkFailed");
  } finally {
    updateChecking.value = false;
  }
}

async function applyUpdate() {
  if (updateApplying.value) return;
  updateApplying.value = true;
  updateError.value = "";
  updateProgress.value = 0;
  try {
    await updateApply();
    updateDone.value = true;
  } catch (e) {
    updateError.value = updateErrText(e, "updateFailed");
  } finally {
    updateApplying.value = false;
  }
}

function openReleases() {
  void openPath(releasesUrl);
}

// 渲染后的更新日志里的链接：用系统默认浏览器打开，避免在设置 webview 内跳转。
function onNotesClick(e: MouseEvent) {
  const anchor = (e.target as HTMLElement | null)?.closest?.("a") as
    | HTMLAnchorElement
    | null;
  if (!anchor) return;
  const href = anchor.href;
  if (!/^(https?:|mailto:)/i.test(href)) return;
  e.preventDefault();
  e.stopPropagation();
  void openPath(href);
}

async function restartSettingsNow() {
  try {
    await restartSettings();
  } catch {
    /* ignore */
  }
}

listen<{ percentage: number }>("update_download_progress", (e) => {
  updateProgress.value = Math.round(e.payload.percentage);
}).then((un) => {
  unlistenProgress = un;
});
let unlistenProgress: UnlistenFn | null = null;
onBeforeUnmount(() => unlistenProgress?.());
</script>

<template>
  <div v-if="config" class="settings">
    <nav class="tabbar" data-tauri-drag-region>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'general' }"
        @mousedown="onTabDown"
        @click="onTabClick('general', $event)"
      >
        {{ t("settings.tabs.general") }}
      </button>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'integration' }"
        @mousedown="onTabDown"
        @click="onTabClick('integration', $event)"
      >
        {{ t("settings.tabs.integration") }}
      </button>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'channel' }"
        @mousedown="onTabDown"
        @click="onTabClick('channel', $event)"
      >
        {{ t("settings.tabs.channel") }}
      </button>
      <button
        v-if="!isWindows && config.experimental.enabled"
        data-tauri-drag-region
        :class="{ active: activeTab === 'experimental' }"
        @mousedown="onTabDown"
        @click="onTabClick('experimental', $event)"
      >
        {{ t("settings.tabs.experimental") }}
      </button>
    </nav>

    <div class="settings-body">
      <!-- 通用 -->
      <template v-if="activeTab === 'general'">
        <div class="card">
          <p class="card-title">{{ t("settings.appearance.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.appearance.theme") }}</span>
            <span class="spacer"></span>
            <div class="segmented">
              <button
                :class="{ active: config.general.theme === 'system' }"
                @click="changeTheme('system')"
              >
                {{ t("settings.appearance.themeSystem") }}
              </button>
              <button
                :class="{ active: config.general.theme === 'light' }"
                @click="changeTheme('light')"
              >
                {{ t("settings.appearance.themeLight") }}
              </button>
              <button
                :class="{ active: config.general.theme === 'dark' }"
                @click="changeTheme('dark')"
              >
                {{ t("settings.appearance.themeDark") }}
              </button>
            </div>
          </div>
          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.appearance.language") }}</span>
            <span class="spacer"></span>
            <select
              class="select"
              :value="config.general.language"
              @change="changeLanguage(($event.target as HTMLSelectElement).value as UiLanguage)"
            >
              <option value="auto">
                {{ t("settings.appearance.languageSystem") }}
              </option>
              <option value="en">English</option>
              <option value="zh">简体中文</option>
            </select>
          </div>
        </div>

        <div class="card">
          <p class="card-title">{{ t("settings.popupBehavior.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.popupBehavior.alwaysOnTop") }}</span>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.general.alwaysOnTop"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>
          <template v-if="isMac && glassSupported">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{
                t("settings.popupBehavior.windowEffect")
              }}</span>
              <span class="spacer"></span>
              <div class="segmented">
                <button
                  :class="{ active: config.general.windowEffect === 'glass' }"
                  @click="changeWindowEffect('glass')"
                >
                  {{ t("settings.popupBehavior.effectGlass") }}
                </button>
                <button
                  :class="{ active: config.general.windowEffect === 'blur' }"
                  @click="changeWindowEffect('blur')"
                >
                  {{ t("settings.popupBehavior.effectBlur") }}
                </button>
              </div>
            </div>
          </template>
          <template v-if="isMac">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{
                t("settings.popupBehavior.appearAnimation")
              }}</span>
              <span class="spacer"></span>
              <div class="segmented">
                <button
                  :class="{ active: config.general.appearAnimation === 'none' }"
                  @click="changeAnimation('none')"
                >
                  None
                </button>
                <button
                  :class="{
                    active: config.general.appearAnimation === 'document',
                  }"
                  @click="changeAnimation('document')"
                >
                  Document
                </button>
                <button
                  :class="{ active: config.general.appearAnimation === 'alert' }"
                  @click="changeAnimation('alert')"
                >
                  Alert
                </button>
              </div>
            </div>
          </template>
          <template v-if="soundSupport.kind !== 'none'">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{ t("settings.popupBehavior.sound") }}</span>
              <span class="spacer"></span>
              <select
                class="select"
                :value="config.general.popupSound"
                @change="changePopupSound(($event.target as HTMLSelectElement).value)"
              >
                <option value="">{{ t("settings.popupBehavior.soundOff") }}</option>
                <template v-if="soundSupport.kind === 'named'">
                  <option v-for="n in soundSupport.names" :key="n" :value="n">{{ n }}</option>
                </template>
                <option v-else value="default">{{ t("settings.popupBehavior.soundOn") }}</option>
              </select>
              <button
                class="btn"
                type="button"
                style="margin-left: 6px"
                :disabled="!config.general.popupSound"
                @click="previewSound"
              >
                {{ t("settings.popupBehavior.soundPreview") }}
              </button>
            </div>
          </template>
          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.popupBehavior.testPopup") }}</span>
            <span class="spacer"></span>
            <button class="btn" type="button" @click="openTestPopup">
              {{ t("common.test") }}
            </button>
          </div>
        </div>

        <!-- 菜单栏图标（仅 macOS/Linux 桌面；Windows 不支持） -->
        <div v-if="!isWindows" class="card">
          <p class="card-title">{{ t("settings.menuBar.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.menuBar.icon") }}</span>
            <span class="spacer"></span>
            <div class="segmented">
              <button
                :class="{ active: config.general.menuBarIcon === 'off' }"
                @click="changeMenuBarIcon('off')"
              >
                {{ t("settings.menuBar.off") }}
              </button>
              <button
                :class="{ active: config.general.menuBarIcon === 'active' }"
                @click="changeMenuBarIcon('active')"
              >
                {{ t("settings.menuBar.active") }}
              </button>
              <button
                :class="{ active: config.general.menuBarIcon === 'always' }"
                @click="changeMenuBarIcon('always')"
              >
                {{ t("settings.menuBar.always") }}
              </button>
            </div>
          </div>
          <p class="card-desc">{{ t("settings.menuBar.hint") }}</p>
        </div>

        <!-- 回复历史 -->
        <div class="card">
          <p class="card-title">{{ t("settings.history.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.history.limit") }}</span>
            <span class="spacer"></span>
            <input
              class="input num"
              type="number"
              min="0"
              step="1"
              :value="config.general.historyLimit"
              @change="changeHistoryLimit(Number(($event.target as HTMLInputElement).value))"
            />
          </div>
          <p class="card-desc">{{ t("settings.history.limitHint") }}</p>
          <template v-if="overLimit">
            <hr class="divider" />
            <div class="row">
              <span class="result err">{{ t("settings.history.overLimit") }}</span>
              <span class="spacer"></span>
              <button class="btn" type="button" @click="cleanHistoryNow">
                {{ t("settings.history.cleanNow") }}
              </button>
            </div>
          </template>
        </div>

        <!-- 语音输入（仅 macOS） -->
        <div v-if="isMac" class="card">
          <p class="card-title">{{ t("settings.speech.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.speech.language") }}</span>
            <span class="spacer"></span>
            <select
              class="select"
              :value="config.general.speechLanguage"
              @change="changeSpeechLanguage(($event.target as HTMLSelectElement).value)"
            >
              <option
                v-for="lang in SPEECH_LANGUAGES"
                :key="lang.value"
                :value="lang.value"
              >
                {{
                  lang.value === "auto"
                    ? t("settings.speech.languageSystem")
                    : lang.label
                }}
              </option>
            </select>
          </div>
          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.speech.shortcut") }}</span>
            <span class="spacer"></span>
            <button
              class="btn shortcut-rec"
              :class="{ recording: recordingShortcut }"
              type="button"
              @click="startRecordShortcut"
            >
              {{
                recordingShortcut
                  ? shortcutPreview || t("settings.speech.recording")
                  : config.general.speechShortcut
                  ? formatShortcut(config.general.speechShortcut)
                  : t("shortcut.none")
              }}
            </button>
            <button
              class="btn"
              type="button"
              style="margin-left: 6px"
              :disabled="!config.general.speechShortcut && !recordingShortcut"
              @click="clearShortcut"
            >
              {{ t("settings.speech.clear") }}
            </button>
          </div>
          <p v-if="shortcutError" class="result err">
            {{ t("shortcut.conflict." + shortcutError.key, shortcutError.params || {}) }}
          </p>
          <p
            v-else-if="recordingShortcut"
            class="card-desc"
            style="margin-top: 6px"
          >
            {{ t("settings.speech.recordHint") }}
          </p>
        </div>

        <!-- 关于 / 版本自更新 -->
        <div class="card">
          <p class="card-title">{{ t("settings.about.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.about.currentVersion") }}</span>
            <span class="spacer"></span>
            <span class="value">{{ appVersion || "—" }}</span>
          </div>
          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.about.latestVersion") }}</span>
            <span class="spacer"></span>
            <span class="value" v-if="updateInfo && !updateChecking">
              {{ updateInfo.latestVersion }}
              <template v-if="!updateInfo.available">
                · {{ t("settings.about.upToDate") }}</template
              >
            </span>
            <span class="value" v-else-if="updateChecking">{{
              t("settings.about.checking")
            }}</span>
            <span class="value" v-else>—</span>
            <button
              class="btn"
              type="button"
              style="margin-left: 8px"
              :disabled="updateChecking"
              @click="checkUpdate(true)"
            >
              {{ t("settings.about.check") }}
            </button>
          </div>

          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.about.currentNotesTitle") }}</span>
            <span class="spacer"></span>
            <a class="link" href="#" @click.prevent="toggleCurrentNotes">{{
              currentNotesOpen
                ? t("settings.about.hideCurrentNotes")
                : t("settings.about.viewCurrentNotes")
            }}</a>
          </div>
          <template v-if="currentNotesOpen">
            <p v-if="currentNotesLoading" class="card-desc">
              {{ t("settings.about.notesLoading") }}
            </p>
            <p v-else-if="currentNotesError" class="result err">
              {{ currentNotesError }}
            </p>
            <div
              v-else-if="currentNotesHtml"
              class="release-notes markdown"
              v-html="currentNotesHtml"
              @click="onNotesClick"
            ></div>
            <p v-else class="card-desc">{{ t("settings.about.noNotes") }}</p>
          </template>

          <template v-if="updateInfo && updateInfo.available">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{
                t("settings.about.updateAvailable", {
                  version: updateInfo.latestVersion,
                })
              }}</span>
              <span class="spacer"></span>
              <button
                v-if="!updateDone"
                class="btn btn-primary"
                type="button"
                :disabled="updateApplying"
                @click="applyUpdate"
              >
                {{
                  updateApplying
                    ? updateProgress > 0
                      ? `${t("settings.about.updating")} ${updateProgress}%`
                      : t("settings.about.updating")
                    : t("settings.about.update")
                }}
              </button>
              <button
                v-else
                class="btn btn-primary"
                type="button"
                @click="restartSettingsNow"
              >
                {{ t("settings.about.restartSettings") }}
              </button>
            </div>
            <p class="card-desc">
              {{
                updateDone
                  ? t("settings.about.updatedRestartHint")
                  : t("settings.about.applyAfterAnswer")
              }}
            </p>

            <template v-if="notesHtml">
              <hr class="divider" />
              <p class="label">{{ t("settings.about.releaseNotes") }}</p>
              <div
                class="release-notes markdown"
                v-html="notesHtml"
                @click="onNotesClick"
              ></div>
            </template>
            <div class="row" style="margin-top: 8px">
              <span class="spacer"></span>
              <a class="link" href="#" @click.prevent="openReleases">{{
                t("settings.about.viewAllReleases")
              }}</a>
            </div>
          </template>

          <p v-if="updateError" class="result err" style="margin-top: 8px">
            {{ updateError }}
          </p>
        </div>

        <!-- 隐蔽开关：实验性功能（Windows 不显示） -->
        <div v-if="!isWindows" class="card experimental-toggle">
          <div class="row">
            <div class="col">
              <span class="label">{{ t("settings.experimental.enableLabel") }}</span>
              <p class="card-desc">{{ t("settings.experimental.enableHint") }}</p>
            </div>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.experimental.enabled"
                @change="toggleExperimental"
              />
              <span class="track"></span>
            </label>
          </div>
        </div>
      </template>

      <!-- 实验性高级功能 -->
      <template v-else-if="activeTab === 'experimental'">
        <div class="card">
          <p class="card-title">{{ t("settings.experimental.lifecycleTitle") }}</p>
          <p class="card-desc">{{ t("settings.experimental.lifecycleDesc") }}</p>
          <hr class="divider" />
          <template v-for="(kind, i) in LIFECYCLE_KINDS" :key="kind">
            <hr v-if="i > 0" class="divider" />
            <div class="row">
              <div class="col">
                <span class="label">{{ lifecycleLabel(kind) }}</span>
                <p
                  v-if="!lifecycleStatus[kind].supported"
                  class="card-desc"
                >
                  {{ t("settings.experimental.unsupported") }}
                </p>
                <p
                  v-else-if="lifecycleStatus[kind].outdated"
                  class="card-desc warn"
                >
                  {{ t("settings.experimental.outdated") }}
                </p>
                <p
                  v-else-if="lifecycleError[kind]"
                  class="card-desc err"
                >
                  {{ lifecycleError[kind] }}
                </p>
              </div>
              <span class="spacer"></span>
              <label class="switch">
                <input
                  type="checkbox"
                  :checked="lifecycleStatus[kind].installed"
                  :disabled="
                    !lifecycleStatus[kind].supported || lifecycleBusy[kind]
                  "
                  @change="
                    toggleLifecycle(
                      kind,
                      ($event.target as HTMLInputElement).checked
                    )
                  "
                />
                <span class="track"></span>
              </label>
            </div>
          </template>
        </div>

        <!-- IM 渠道按需发送（从「渠道」Tab 迁来，归入实验区；配置键仍为 autoActivation） -->
        <div class="card">
          <div class="row">
            <p class="card-title">
              {{ t("settings.channels.autoActivationTitle") }}
            </p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.autoActivation"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>
          <p class="card-desc">
            {{ t("settings.channels.autoActivationDesc") }}
          </p>
          <p class="card-desc hint">
            {{ t("settings.channels.autoActivationLifecycleHint") }}
          </p>
        </div>
      </template>

      <!-- Agent -->
      <template v-else-if="activeTab === 'integration'">
        <div
          v-if="openMenuKey"
          class="menu-backdrop"
          @click="closeOpenMenu"
        ></div>
        <p class="section-intro">{{ t("settings.integration.overviewDesc") }}</p>

        <!-- 手动集成：参考提示词（CLI / MCP 双版本 + MCP 配置示例） -->
        <p class="section-title">{{ t("settings.integration.manualTitle") }}</p>
        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.integration.promptTitle") }}</p>
            <span class="spacer"></span>
            <div class="segmented">
              <button
                type="button"
                class="seg"
                :class="{ active: promptVariant === 'cli' }"
                @click="setPromptVariant('cli')"
              >
                {{ t("settings.integration.modeCli") }}
              </button>
              <button
                type="button"
                class="seg"
                :class="{ active: promptVariant === 'mcp' }"
                @click="setPromptVariant('mcp')"
              >
                {{ t("settings.integration.modeMcp") }}
              </button>
            </div>
            <button class="btn" type="button" @click="copyPrompt">
              {{
                promptCopied
                  ? t("settings.integration.copied")
                  : t("settings.integration.copy")
              }}
            </button>
          </div>
          <pre class="code-area">{{ prompt }}</pre>

          <template v-if="promptVariant === 'mcp'">
            <hr class="divider" />
            <p class="card-desc agent-hint">
              {{ t("settings.integration.mcpExampleHint") }}
            </p>
            <div class="row mcp-example-head">
              <p class="label mcp-example-label">
                {{ t("settings.integration.mcpExampleJson") }}
              </p>
              <button
                class="btn"
                type="button"
                @click="copyMcpExample('json')"
              >
                {{
                  mcpJsonCopied
                    ? t("settings.integration.copied")
                    : t("settings.integration.copy")
                }}
              </button>
            </div>
            <pre class="code-area">{{ mcpExampleJson }}</pre>
            <p class="card-desc agent-hint">
              {{ t("settings.integration.mcpTimeoutNote") }}
            </p>
            <div class="row mcp-example-head">
              <p class="label mcp-example-label">
                {{ t("settings.integration.mcpExampleToml") }}
              </p>
              <button
                class="btn"
                type="button"
                @click="copyMcpExample('toml')"
              >
                {{
                  mcpTomlCopied
                    ? t("settings.integration.copied")
                    : t("settings.integration.copy")
                }}
              </button>
            </div>
            <pre class="code-area">{{ mcpExampleToml }}</pre>
          </template>
        </div>

        <!-- 自动集成：每个 Agent 一张卡，CLI | MCP | 未集成 三态切换 -->
        <p class="section-title">{{ t("settings.integration.autoTitle") }}</p>
        <div v-for="a in AGENTS" :key="a.id" class="card agent-card">
          <div class="row agent-row">
            <p class="card-title">{{ a.title }}</p>
            <span class="spacer"></span>
            <div class="segmented">
              <button
                type="button"
                class="seg"
                :class="{ active: modes[a.id].mode === 'cli' }"
                :disabled="modeBusy[a.id]"
                @click="setMode(a.id, 'cli')"
              >
                {{ t("settings.integration.modeCli")
                }}<span v-if="a.recommended === 'cli'" class="seg-rec">{{
                  t("settings.integration.recommendedTag")
                }}</span>
              </button>
              <button
                type="button"
                class="seg"
                :class="{ active: modes[a.id].mode === 'mcp' }"
                :disabled="modeBusy[a.id]"
                @click="setMode(a.id, 'mcp')"
              >
                {{ t("settings.integration.modeMcp")
                }}<span v-if="a.recommended === 'mcp'" class="seg-rec">{{
                  t("settings.integration.recommendedTag")
                }}</span>
              </button>
              <button
                type="button"
                class="seg"
                :class="{ active: modes[a.id].mode === 'none' }"
                :disabled="modeBusy[a.id]"
                @click="setMode(a.id, 'none')"
              >
                {{ t("settings.integration.modeNone") }}
              </button>
            </div>
          </div>

          <template v-if="modes[a.id].mode !== 'none'">
            <hr class="divider" />

            <!-- Rules（CLI / MCP 共有） -->
            <div class="row agent-row">
              <span class="label">{{ t("settings.integration.rulesLabel") }}</span>
              <span class="badge">
                <span
                  class="dot"
                  :class="modes[a.id].ruleInstalled ? 'on' : 'off'"
                ></span>
                {{
                  modes[a.id].ruleInstalled
                    ? t("settings.integration.installed")
                    : t("settings.integration.notInstalled")
                }}
              </span>
              <span class="spacer"></span>
              <div v-if="modes[a.id].ruleInstalled" class="menu-wrap">
                <button
                  class="btn"
                  type="button"
                  @click.stop="toggleOpenMenu(a.id + ':rule')"
                >
                  {{ t("settings.integration.openFile") }}
                </button>
                <div v-if="openMenuKey === a.id + ':rule'" class="menu-pop">
                  <button
                    class="menu-item"
                    type="button"
                    @click="revealFile(a.id, 'rule')"
                  >
                    {{ revealLabel }}
                  </button>
                  <button
                    class="menu-item"
                    type="button"
                    @click="openFile(a.id, 'rule')"
                  >
                    {{ t("settings.integration.openFileAction") }}
                  </button>
                </div>
              </div>
            </div>
            <p v-if="modes[a.id].rulePath" class="agent-path">
              {{ modes[a.id].rulePath }}
            </p>
            <p v-if="a.id === 'cursor'" class="card-desc agent-hint">
              {{ t("settings.integration.cursorRulesHint") }}
            </p>

            <!-- CLI 模式：超时 Hook（Codex 无 Hook 给提示） -->
            <template v-if="modes[a.id].mode === 'cli'">
              <hr class="divider" />
              <template v-if="a.hasTimeoutHook">
                <div class="row agent-row">
                  <span class="label">{{
                    t("settings.integration.hookLabel")
                  }}</span>
                  <span class="badge">
                    <span
                      class="dot"
                      :class="modes[a.id].timeoutHookInstalled ? 'on' : 'off'"
                    ></span>
                    {{
                      modes[a.id].timeoutHookInstalled
                        ? t("settings.integration.installed")
                        : t("settings.integration.notInstalled")
                    }}
                  </span>
                  <span class="spacer"></span>
                  <div v-if="modes[a.id].timeoutHookInstalled" class="menu-wrap">
                    <button
                      class="btn"
                      type="button"
                      @click.stop="toggleOpenMenu(a.id + ':hook')"
                    >
                      {{ t("settings.integration.openFile") }}
                    </button>
                    <div v-if="openMenuKey === a.id + ':hook'" class="menu-pop">
                      <button
                        class="menu-item"
                        type="button"
                        @click="revealFile(a.id, 'hook')"
                      >
                        {{ revealLabel }}
                      </button>
                      <button
                        class="menu-item"
                        type="button"
                        @click="openFile(a.id, 'hook')"
                      >
                        {{ t("settings.integration.openFileAction") }}
                      </button>
                    </div>
                  </div>
                </div>
                <p class="card-desc agent-hint">
                  {{ t("settings.integration.hookShort") }}
                </p>
                <p
                  v-if="!modes[a.id].timeoutHookSupported"
                  class="result err"
                >
                  {{ t("settings.integration.windowsUnsupported") }}
                </p>
              </template>
              <p v-else class="card-desc agent-hint">
                {{ t("settings.integration.codexNoHook") }}
              </p>
            </template>

            <!-- MCP 模式：MCP 配置 -->
            <template v-if="modes[a.id].mode === 'mcp'">
              <hr class="divider" />
              <div class="row agent-row">
                <span class="label">{{
                  t("settings.integration.mcpConfigLabel")
                }}</span>
                <span class="badge">
                  <span
                    class="dot"
                    :class="modes[a.id].mcpConfigInstalled ? 'on' : 'off'"
                  ></span>
                  {{
                    modes[a.id].mcpConfigInstalled
                      ? t("settings.integration.installed")
                      : t("settings.integration.notInstalled")
                  }}
                </span>
                <span class="spacer"></span>
                <div v-if="modes[a.id].mcpConfigInstalled" class="menu-wrap">
                  <button
                    class="btn"
                    type="button"
                    @click.stop="toggleOpenMenu(a.id + ':mcp')"
                  >
                    {{ t("settings.integration.openFile") }}
                  </button>
                  <div v-if="openMenuKey === a.id + ':mcp'" class="menu-pop">
                    <button
                      class="menu-item"
                      type="button"
                      @click="revealFile(a.id, 'mcp')"
                    >
                      {{ revealLabel }}
                    </button>
                    <button
                      class="menu-item"
                      type="button"
                      @click="openFile(a.id, 'mcp')"
                    >
                      {{ t("settings.integration.openFileAction") }}
                    </button>
                  </div>
                </div>
              </div>
              <p v-if="modes[a.id].mcpConfigPath" class="agent-path">
                {{ modes[a.id].mcpConfigPath }}
              </p>
              <p class="card-desc agent-hint">
                {{ t("settings.integration.mcpModeHint") }}
              </p>
            </template>

            <!-- 更新（产物落后于最新版本时） -->
            <div v-if="modes[a.id].needsUpdate" class="row agent-row">
              <span class="spacer"></span>
              <button
                class="btn btn-update"
                type="button"
                :disabled="modeBusy[a.id]"
                @click="updateMode(a.id)"
              >
                <span class="dot-update"></span
                >{{ t("settings.integration.update") }}
              </button>
            </div>
          </template>

          <p
            v-if="modeMessage[a.id]"
            class="result"
            :class="modeError[a.id] ? 'err' : 'ok'"
          >
            {{ modeMessage[a.id] }}
          </p>
        </div>
      </template>

      <!-- 通信渠道 -->
      <template v-else>
        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.popupTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.popup.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.popup.enabled">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{ t("settings.channels.rememberSize") }}</span>
              <span class="spacer"></span>
              <label class="switch">
                <input
                  type="checkbox"
                  v-model="config.channels.popup.rememberSize"
                  @change="persist"
                />
                <span class="track"></span>
              </label>
            </div>
            <div class="row">
              <span class="label">{{ t("settings.channels.defaultWidth") }}</span>
              <span class="spacer"></span>
              <div class="stepper">
                <button
                  type="button"
                  :disabled="config.channels.popup.width <= 360"
                  @click="stepWidth(-20)"
                >
                  −
                </button>
                <span class="val">{{ Math.round(config.channels.popup.width) }}</span>
                <button
                  type="button"
                  :disabled="config.channels.popup.width >= 1200"
                  @click="stepWidth(20)"
                >
                  +
                </button>
              </div>
            </div>
            <div class="row">
              <span class="label">{{ t("settings.channels.defaultHeight") }}</span>
              <span class="spacer"></span>
              <div class="stepper">
                <button
                  type="button"
                  :disabled="config.channels.popup.height <= 360"
                  @click="stepHeight(-20)"
                >
                  −
                </button>
                <span class="val">{{
                  Math.round(config.channels.popup.height)
                }}</span>
                <button
                  type="button"
                  :disabled="config.channels.popup.height >= 1400"
                  @click="stepHeight(20)"
                >
                  +
                </button>
              </div>
            </div>
          </template>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.telegramTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.telegram.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.telegram.enabled">
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.botToken") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  type="password"
                  :placeholder="
                    secretsPresent.telegramToken ? SECRET_PLACEHOLDER : ''
                  "
                  v-model="config.channels.telegram.botToken"
                  @change="persist"
                />
                <button
                  v-if="secretsPresent.telegramToken"
                  class="btn"
                  type="button"
                  @click="clearSecret('telegram')"
                >
                  {{ t("settings.channels.clearSecret") }}
                </button>
              </div>
            </div>
            <div class="field">
              <label>{{ t("settings.channels.chatId") }}</label>
              <input
                class="input"
                v-model="config.channels.telegram.chatId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.apiBaseUrl") }}</label>
              <input
                class="input"
                v-model="config.channels.telegram.apiBaseUrl"
                @change="persist"
              />
            </div>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="telegramTesting"
                @click="runTelegramTest"
              >
                {{
                  telegramTesting
                    ? t("settings.channels.testing")
                    : t("settings.channels.testConnection")
                }}
              </button>
              <span class="spacer"></span>
            </div>
            <p
              v-if="telegramMessage"
              class="result"
              :class="telegramError ? 'err' : 'ok'"
            >
              {{ telegramMessage }}
            </p>
          </template>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.dingtalkTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.dingding.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.dingding.enabled">
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.clientId") }}</label>
              <input
                class="input"
                v-model="config.channels.dingding.clientId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.clientSecret") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  type="password"
                  :placeholder="
                    secretsPresent.dingdingSecret ? SECRET_PLACEHOLDER : ''
                  "
                  v-model="config.channels.dingding.clientSecret"
                  @change="persist"
                />
                <button
                  v-if="secretsPresent.dingdingSecret"
                  class="btn"
                  type="button"
                  @click="clearSecret('dingding')"
                >
                  {{ t("settings.channels.clearSecret") }}
                </button>
              </div>
            </div>
            <div class="field">
              <label>{{ t("settings.channels.userId") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  v-model="config.channels.dingding.userId"
                  @change="persist"
                />
                <button
                  class="btn"
                  type="button"
                  :disabled="dingtalkDetecting"
                  @click="runDingtalkDetect"
                >
                  {{
                    dingtalkDetecting
                      ? t("settings.channels.detecting")
                      : t("settings.channels.autoDetect")
                  }}
                </button>
                <button
                  v-if="dingtalkDetecting"
                  class="btn"
                  type="button"
                  @click="cancelDetect"
                >
                  {{ t("settings.channels.detectCancel") }}
                </button>
              </div>
            </div>
            <i18n-t
              v-if="dingtalkDetectCode"
              keypath="settings.channels.detectHint"
              tag="p"
              class="result ok"
            >
              <template #code><b>{{ dingtalkDetectCode }}</b></template>
            </i18n-t>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="dingtalkTesting"
                @click="runDingtalkTest"
              >
                {{
                  dingtalkTesting
                    ? t("settings.channels.testing")
                    : t("settings.channels.testConnection")
                }}
              </button>
              <span class="spacer"></span>
            </div>
            <p
              v-if="dingtalkMessage"
              class="result"
              :class="dingtalkError ? 'err' : 'ok'"
            >
              {{ dingtalkMessage }}
            </p>
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.cardTemplateId") }}</label>
              <input
                class="input"
                v-model="config.channels.dingding.cardTemplateId"
                :placeholder="t('settings.channels.cardTemplateIdPlaceholder')"
                @change="persist"
              />
            </div>
            <hr class="divider" />
            <div class="row">
              <label>{{ t("settings.channels.inlineSmallText") }}</label>
              <span class="spacer"></span>
              <label class="switch">
                <input
                  type="checkbox"
                  v-model="config.channels.dingding.inlineSmallText"
                  @change="persist"
                />
                <span class="track"></span>
              </label>
            </div>
            <p class="card-desc" style="margin-top: 0">
              {{ t("settings.channels.inlineSmallTextHint") }}
            </p>
            <div class="row">
              <label>{{ t("settings.channels.convertTextToDocx") }}</label>
              <span class="spacer"></span>
              <label class="switch">
                <input
                  type="checkbox"
                  v-model="config.channels.dingding.convertTextToDocx"
                  @change="persist"
                />
                <span class="track"></span>
              </label>
            </div>
            <p class="card-desc" style="margin-top: 0">
              {{ t("settings.channels.convertTextToDocxHint") }}
            </p>
          </template>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.feishuTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.feishu.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.feishu.enabled">
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.appId") }}</label>
              <input
                class="input"
                v-model="config.channels.feishu.appId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.appSecret") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  type="password"
                  :placeholder="
                    secretsPresent.feishuSecret ? SECRET_PLACEHOLDER : ''
                  "
                  v-model="config.channels.feishu.appSecret"
                  @change="persist"
                />
                <button
                  v-if="secretsPresent.feishuSecret"
                  class="btn"
                  type="button"
                  @click="clearSecret('feishu')"
                >
                  {{ t("settings.channels.clearSecret") }}
                </button>
              </div>
            </div>
            <div class="field">
              <label>{{ t("settings.channels.openId") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  v-model="config.channels.feishu.openId"
                  @change="persist"
                />
                <button
                  class="btn"
                  type="button"
                  :disabled="feishuDetecting"
                  @click="runFeishuDetect"
                >
                  {{
                    feishuDetecting
                      ? t("settings.channels.detecting")
                      : t("settings.channels.autoDetect")
                  }}
                </button>
                <button
                  v-if="feishuDetecting"
                  class="btn"
                  type="button"
                  @click="cancelDetect"
                >
                  {{ t("settings.channels.detectCancel") }}
                </button>
              </div>
            </div>
            <i18n-t
              v-if="feishuDetectCode"
              keypath="settings.channels.feishuDetectHint"
              tag="p"
              class="result ok"
            >
              <template #code><b>{{ feishuDetectCode }}</b></template>
            </i18n-t>
            <div class="field">
              <label>{{ t("settings.channels.feishuBaseUrl") }}</label>
              <input
                class="input"
                v-model="config.channels.feishu.baseUrl"
                :placeholder="t('settings.channels.feishuBaseUrlPlaceholder')"
                @change="persist"
              />
            </div>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="feishuTesting"
                @click="runFeishuTest"
              >
                {{
                  feishuTesting
                    ? t("settings.channels.testing")
                    : t("settings.channels.testConnection")
                }}
              </button>
              <span class="spacer"></span>
            </div>
            <p
              v-if="feishuMessage"
              class="result"
              :class="feishuError ? 'err' : 'ok'"
            >
              {{ feishuMessage }}
            </p>
          </template>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.slackTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.slack.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.slack.enabled">
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.slackBotToken") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  type="password"
                  :placeholder="
                    secretsPresent.slackBotToken ? SECRET_PLACEHOLDER : 'xoxb-…'
                  "
                  v-model="config.channels.slack.botToken"
                  @change="persist"
                />
                <button
                  v-if="secretsPresent.slackBotToken"
                  class="btn"
                  type="button"
                  @click="clearSecret('slackBot')"
                >
                  {{ t("settings.channels.clearSecret") }}
                </button>
              </div>
            </div>
            <div class="field">
              <label>{{ t("settings.channels.slackAppToken") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  type="password"
                  :placeholder="
                    secretsPresent.slackAppToken ? SECRET_PLACEHOLDER : 'xapp-…'
                  "
                  v-model="config.channels.slack.appToken"
                  @change="persist"
                />
                <button
                  v-if="secretsPresent.slackAppToken"
                  class="btn"
                  type="button"
                  @click="clearSecret('slackApp')"
                >
                  {{ t("settings.channels.clearSecret") }}
                </button>
              </div>
            </div>
            <div class="field">
              <label>{{ t("settings.channels.slackUserId") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  v-model="config.channels.slack.userId"
                  @change="persist"
                />
                <button
                  class="btn"
                  type="button"
                  :disabled="slackDetecting"
                  @click="runSlackDetect"
                >
                  {{
                    slackDetecting
                      ? t("settings.channels.detecting")
                      : t("settings.channels.autoDetect")
                  }}
                </button>
                <button
                  v-if="slackDetecting"
                  class="btn"
                  type="button"
                  @click="cancelDetect"
                >
                  {{ t("settings.channels.detectCancel") }}
                </button>
              </div>
            </div>
            <i18n-t
              v-if="slackDetectCode"
              keypath="settings.channels.slackDetectHint"
              tag="p"
              class="result ok"
            >
              <template #code><b>{{ slackDetectCode }}</b></template>
            </i18n-t>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="slackTesting"
                @click="runSlackTest"
              >
                {{
                  slackTesting
                    ? t("settings.channels.testing")
                    : t("settings.channels.testConnection")
                }}
              </button>
              <span class="spacer"></span>
            </div>
            <p
              v-if="slackMessage"
              class="result"
              :class="slackError ? 'err' : 'ok'"
            >
              {{ slackMessage }}
            </p>
          </template>
        </div>

        <div class="card">
          <p class="card-desc" style="margin: 0">
            {{ t("settings.channels.moreSoon") }}
          </p>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.settings {
  position: relative;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
/* 实验区：标签 + 说明纵向堆叠的行内列 */
.row .col {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.row .col .card-desc {
  margin: 0;
}
.card-desc.err {
  color: #ff453a;
}
.card-desc.warn {
  color: #ff9f0a;
}
/* 关联建议：比正文稍弱、与上方说明留出间距 */
.card-desc.hint {
  margin-top: 6px;
  opacity: 0.85;
}
/* 关于区：版本值、发布链接、更新日志容器 */
.row .value {
  font-size: 13px;
  color: var(--text-secondary);
}
.link {
  font-size: 12px;
  color: var(--accent, #3b82f6);
  cursor: pointer;
  text-decoration: none;
}
.link:hover {
  text-decoration: underline;
}
.release-notes {
  max-height: 220px;
  overflow-y: auto;
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-secondary);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border, rgba(127, 127, 127, 0.2));
  border-radius: 8px;
  margin-top: var(--space-2);
}
.release-notes :deep(h1),
.release-notes :deep(h2),
.release-notes :deep(h3) {
  font-size: 13px;
  margin: 8px 0 4px;
}
.release-notes :deep(ul) {
  margin: 4px 0;
  padding-left: 18px;
}
.release-notes :deep(p) {
  margin: 4px 0;
}
.release-notes :deep(a) {
  color: var(--accent, #3b82f6);
}
.settings-body {
  flex: 1 1 auto;
  overflow-y: auto;
  padding: var(--space-4);
}
/* Agent 集成页：顶部原理说明 + 「手动/自动集成」分组标题 */
.section-intro {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.5;
  margin: 0 0 var(--space-4);
}
.section-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-secondary);
  margin: 0 0 var(--space-2);
}
/* 标题前若紧邻上一组的卡片，补足分组间距 */
.card + .section-title {
  margin-top: var(--space-4);
}
/* 删除 promptDesc 后，参考提示词与上方标题行需补回间距 */
.code-area {
  margin-top: var(--space-3);
}
/* Agent 分组卡：压缩留白，避免页面过高 */
.agent-card {
  padding-top: var(--space-3);
  padding-bottom: var(--space-3);
}
.agent-card .card-title {
  margin-bottom: var(--space-2);
}
.agent-card .agent-row {
  min-height: 28px;
}
.agent-card .divider {
  margin: var(--space-2) 0;
}
/* 文件路径：等宽小字，可断行 */
.agent-path {
  margin: 4px 0 0;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-size: 11px;
  color: var(--text-secondary, #888);
  word-break: break-all;
}
.agent-hint {
  margin-top: 4px;
}
.badge.muted {
  opacity: 0.55;
}

/* 三态切换：CLI | MCP | 未集成（也用于参考提示词的 CLI/MCP 切换） */
.segmented {
  display: inline-flex;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-subtle, rgba(127, 127, 127, 0.08));
}
.segmented .seg {
  padding: 3px 12px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary, #888);
  font-size: 12px;
  line-height: 18px;
  white-space: nowrap;
  cursor: pointer;
}
.segmented .seg:hover:not(.active):not(:disabled) {
  color: var(--text-primary);
}
.segmented .seg.active {
  background: var(--accent);
  color: #fff;
}
.segmented .seg:disabled {
  cursor: default;
  opacity: 0.6;
}
/* 推荐档位标记：小号绿色胶囊；激活态（蓝底）切白色胶囊保证对比度 */
.segmented .seg .seg-rec {
  margin-left: 5px;
  padding: 0 5px;
  border-radius: 999px;
  font-size: 10px;
  line-height: 15px;
  color: var(--rec-badge-fg);
  background: color-mix(in srgb, var(--accent-green) 22%, transparent);
}
.segmented .seg.active .seg-rec {
  color: #fff;
  background: color-mix(in srgb, #fff 28%, transparent);
}
.mcp-example-label {
  margin: var(--space-2) 0 4px;
}
.mcp-example-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}
.mcp-example-head .mcp-example-label {
  margin: var(--space-2) 0 4px;
}

/* 「更新」按钮：边框/背景同普通按钮，仅文字与前置圆点用橙色提示有更新 */
.btn-update {
  display: inline-flex;
  align-items: center;
  white-space: nowrap;
  color: var(--accent-orange);
}
.btn-update .dot-update {
  flex: none;
  width: 7px;
  height: 7px;
  margin-right: 5px;
  border-radius: 50%;
  background: var(--accent-orange);
}

/* 「打开」下拉菜单：在文件管理器中显示 / 打开文件 */
.menu-wrap {
  position: relative;
  display: inline-block;
}
.menu-backdrop {
  position: fixed;
  inset: 0;
  z-index: 50;
}
.menu-pop {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  z-index: 60;
  min-width: 168px;
  padding: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg);
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.18);
}
.menu-item {
  display: block;
  width: 100%;
  padding: 6px 10px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-primary);
  font-size: 13px;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
}
.menu-item:hover {
  background: var(--accent);
  color: #fff;
}

/* 快捷键录制按钮：等宽、最小宽度，录制态用强调色描边 */
.shortcut-rec {
  min-width: 76px;
  font-variant-numeric: tabular-nums;
}
.shortcut-rec.recording {
  border-color: var(--accent);
  color: var(--accent);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 30%, transparent);
}
</style>
