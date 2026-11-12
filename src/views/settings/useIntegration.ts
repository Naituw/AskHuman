// 「Agent 集成」域：各 Agent 的 CLI/MCP/未集成三态、单项与批量更新、
// 停止确认与权限确认开关、参考提示词与 MCP 手动配置示例。
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  agentModeStatus,
  agentModeSet,
  agentModeUpdate,
  agentModeUpdateArtifact,
  agentPermissionSet,
  agentAskQuestionSet,
  agentStopSet,
  agentDelegatedSet,
  agentLifecycleInstall,
  agentLifecycleUninstall,
  agentRuleReveal,
  agentRuleOpen,
  mcpConfigReveal,
  mcpConfigOpen,
  mcpCommandPath,
  agentHookReveal,
  agentHookOpen,
  getPrompt,
  collaborationStyleDefaults,
  saveCollaborationStyle,
} from "../../lib/ipc";
import type {
  AgentId,
  AgentMode,
  AgentModeStatus,
  AgentTaskReadiness,
  CollaborationStyle,
  CollaborationStyleApplyResult,
} from "../../lib/types";
import { isMac, isWindows, supportsAgentTasks } from "../../lib/platform";
import type { SettingsCore } from "./context";
import { useAgentDetails } from "./useAgentDetails";

const PI_MINIMUM_VERSION = "0.82.0";

export function useIntegration(
  core: SettingsCore,
  tasks: {
    refreshAgentTaskSettings: (scan?: boolean) => Promise<void>;
    ensurePiReadiness: () => Promise<AgentTaskReadiness | undefined>;
    ensureCodexReadiness: () => Promise<AgentTaskReadiness | undefined>;
    taskReadiness: { value: AgentTaskReadiness[] };
  },
) {
  const { t } = useI18n();
  const { config } = core;
  const {
    refreshAgentTaskSettings,
    ensurePiReadiness,
    ensureCodexReadiness,
    taskReadiness,
  } = tasks;

  // 「在文件管理器中显示」的按平台措辞（访达 / 文件资源管理器 / 文件管理器），单一来源。
  const revealLabel = computed(() => {
    if (isMac) return t("settings.integration.revealInFinder");
    if (isWindows) return t("settings.integration.revealInExplorer");
    return t("settings.integration.revealInFileManager");
  });

  const prompt = ref("");
  const promptCopied = ref(false);
  // 手动集成卡的提示词变体：CLI 版 / MCP 版（切换即重载正文）。
  const promptVariant = ref<"cli" | "mcp">("cli");
  const promptAgent = ref<AgentId | "">("");
  const promptSupportsCli = computed(() => !promptAgent.value || AGENTS.find((agent) => agent.id === promptAgent.value)?.hasCli);
  const promptSupportsMcp = computed(() => !promptAgent.value || AGENTS.find((agent) => agent.id === promptAgent.value)?.hasMcp);

  // 各 Agent 的展示信息。Codex 没有「超时 Hook」概念（hasTimeoutHook=false），
  // 且无法延长 CLI 超时，故推荐 MCP；Cursor / Claude Code 有可靠超时 Hook，推荐 CLI。
  // `hasCli`：是否提供 CLI 档（超时 Hook 方案）。Grok 的 Composer CLI 会自动后台化，
  // 超时不可靠，故仅 None|Mcp 两态。`instructionKind`：指令载体（规则文件 vs skill）。
  // Codex's carrier is the SessionStart hook ("hook"): the protocol is injected at session start
  // instead of living in a static AGENTS.md block.
  const AGENTS: {
    id: AgentId;
    title: string;
    hasTimeoutHook: boolean;
    hasCli: boolean;
    hasMcp: boolean;
    instructionKind: "rule" | "skill" | "hook";
    recommended: AgentMode;
  }[] = [
    { id: "cursor", title: "Cursor", hasTimeoutHook: true, hasCli: true, hasMcp: true, instructionKind: "rule", recommended: "cli" },
    { id: "claude", title: "Claude Code", hasTimeoutHook: true, hasCli: true, hasMcp: true, instructionKind: "rule", recommended: "cli" },
    { id: "codex", title: "Codex", hasTimeoutHook: false, hasCli: true, hasMcp: true, instructionKind: "hook", recommended: "mcp" },
    { id: "grok", title: "Grok", hasTimeoutHook: false, hasCli: false, hasMcp: true, instructionKind: "skill", recommended: "mcp" },
    { id: "antigravity", title: "Antigravity", hasTimeoutHook: false, hasCli: true, hasMcp: true, instructionKind: "rule", recommended: "cli" },
    { id: "pi", title: "Pi", hasTimeoutHook: true, hasCli: true, hasMcp: false, instructionKind: "rule", recommended: "cli" },
  ];

  const emptyMode = (): AgentModeStatus => ({
    mode: "none",
    needsUpdate: false,
    ruleNeedsUpdate: false,
    hookNeedsUpdate: false,
    mcpNeedsUpdate: false,
    rulePath: "",
    ruleInstalled: false,
    timeoutHookSupported: false,
    timeoutHookInstalled: false,
    timeoutHookNeedsUpdate: false,
    recoveryHookInstalled: false,
    permission: {
      supported: false,
      unsupportedReason: "native_permission_request_unsupported",
      enabled: false,
      configured: false,
      outdated: false,
      needsUpdate: false,
      knownBlockedReason: null,
      otherHandlersDetected: false,
    },
    permissionNeedsUpdate: false,
    lifecycle: {
      enabled: false,
      preferenceConfigured: false,
      installed: false,
      outdated: false,
      supported: true,
      needsUpdate: false,
      cleanupRequired: false,
    },
    stop: {
      supported: false,
      enabled: false,
      installed: false,
      outdated: false,
      otherHandlersDetected: false,
    },
    delegated: { supported: false, enabled: false },
    askQuestion: {
      supported: false,
      enabled: false,
      installed: false,
      outdated: false,
    },
    mcpSupported: false,
    mcpConfigPath: "",
    mcpConfigInstalled: false,
    runtimeArtifactKind: "hook",
    agentVersion: null,
    minimumVersion: null,
    versionSupported: true,
  });
  const modes = ref<Record<AgentId, AgentModeStatus>>({
    cursor: emptyMode(),
    claude: emptyMode(),
    codex: emptyMode(),
    grok: emptyMode(),
    pi: emptyMode(),
    antigravity: { ...emptyMode(), lifecycle: { ...emptyMode().lifecycle, supported: false } },
  });
  const modeBusy = ref<Record<AgentId, boolean>>({
    cursor: false,
    claude: false,
    codex: false,
    grok: false,
    pi: false,
    antigravity: false,
  });
  const modeMessage = ref<Record<AgentId, string | null>>({
    cursor: null,
    claude: null,
    codex: null,
    grok: null,
    pi: null,
    antigravity: null,
  });
  const modeError = ref<Record<AgentId, boolean>>({
    cursor: false,
    claude: false,
    codex: false,
    grok: false,
    pi: false,
    antigravity: false,
  });

  const modeReadError = ref<Partial<Record<AgentId, boolean>>>({});

  function overlayPiVersion(item: AgentTaskReadiness) {
    modes.value.pi = {
      ...modes.value.pi,
      agentVersion: item.version,
      minimumVersion: PI_MINIMUM_VERSION,
      versionSupported: item.binaryReady,
    };
  }

  function overlayPiVersionIfKnown() {
    const item = taskReadiness.value.find((entry) => entry.kind === "pi");
    if (item) overlayPiVersion(item);
  }

  // Codex: the version floor is the one for SessionStart-hook protocol delivery. An unreadable
  // version (Codex not installed) counts as supported, so the card never shows a false alarm.
  function overlayCodexVersion(item: AgentTaskReadiness) {
    modes.value.codex = {
      ...modes.value.codex,
      agentVersion: item.version,
      minimumVersion: item.protocolMinimumVersion ?? null,
      versionSupported: item.protocolSupported !== false,
    };
  }

  function overlayCodexVersionIfKnown() {
    const item = taskReadiness.value.find((entry) => entry.kind === "codex");
    if (item) overlayCodexVersion(item);
  }

  async function refreshMode(agent: AgentId) {
    try {
      modes.value[agent] = await agentModeStatus(agent);
      modeReadError.value[agent] = false;
      if (agent === "pi") { overlayPiVersionIfKnown(); }
      if (agent === "codex") { overlayCodexVersionIfKnown(); }
    } catch (error) {
      modeReadError.value[agent] = true;
      modeMessage.value[agent] = String(error);
      modeError.value[agent] = true;
    }
  }

  const integrationLoading = ref(true);
  const piVersionLoading = ref(false);
  let integrationInflight: Promise<void> | null = null;
  let integrationReady = false;
  let piVersionInflight: Promise<void> | null = null;
  let piVersionReady = false;
  let codexVersionInflight: Promise<void> | null = null;
  let codexVersionReady = false;

  async function ensureIntegration() {
    if (integrationReady) return;
    if (integrationInflight) return integrationInflight;
    const run = initIntegration().finally(() => {
      if (integrationInflight === run) integrationInflight = null;
      integrationReady = true;
      integrationLoading.value = false;
    });
    integrationInflight = run;
    return run;
  }

  async function ensurePiVersion() {
    if (piVersionReady) return;
    if (piVersionInflight) return piVersionInflight;
    const run = (async () => {
      piVersionLoading.value = true;
      try {
        const item = await ensurePiReadiness();
        if (item) overlayPiVersion(item);
      } finally {
        piVersionLoading.value = false;
        piVersionReady = true;
      }
    })();
    piVersionInflight = run.finally(() => {
      if (piVersionInflight === run) piVersionInflight = null;
    });
    return piVersionInflight;
  }

  // Informational only (no spinner, no error surface): the card shows a message only after the
  // probe confirms an old Codex.
  async function ensureCodexVersion() {
    if (codexVersionReady) return;
    if (codexVersionInflight) return codexVersionInflight;
    const run = (async () => {
      try {
        const item = await ensureCodexReadiness();
        if (item) overlayCodexVersion(item);
      } catch {
        /* A failed probe leaves the default "supported" state. */
      } finally {
        codexVersionReady = true;
      }
    })();
    codexVersionInflight = run.finally(() => {
      if (codexVersionInflight === run) codexVersionInflight = null;
    });
    return codexVersionInflight;
  }

  // 一键切换到目标模式（含「未集成」）：自动卸旧装新。
  async function setMode(agent: AgentId, mode: AgentMode) {
    if (modeBusy.value[agent]) return;
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
      if (supportsAgentTasks) await refreshAgentTaskSettings(false);
    }
  }

  async function toggleLifecycle(agent: AgentId, enabled: boolean) {
    if (modeBusy.value[agent]) return;
    modeBusy.value[agent] = true;
    modeMessage.value[agent] = null;
    try {
      if (enabled) await agentLifecycleInstall(agent);
      else await agentLifecycleUninstall(agent);
      modeError.value[agent] = false;
    } catch (e) {
      modeMessage.value[agent] = String(e);
      modeError.value[agent] = true;
    } finally {
      modeBusy.value[agent] = false;
      await refreshMode(agent);
      if (supportsAgentTasks) await refreshAgentTaskSettings(false);
    }
  }

  async function togglePermission(agent: AgentId, enabled: boolean) {
    if (modeBusy.value[agent]) return;
    modeBusy.value[agent] = true;
    modeMessage.value[agent] = null;
    try {
      await agentPermissionSet(agent, enabled);
      modeError.value[agent] = false;
    } catch (e) {
      modeMessage.value[agent] = String(e);
      modeError.value[agent] = true;
    } finally {
      modeBusy.value[agent] = false;
      await refreshMode(agent);
    }
  }

  // 接管 Claude 内置提问工具（spec claude-ask-user-question D3）。
  async function toggleAskQuestion(agent: AgentId, enabled: boolean) {
    if (modeBusy.value[agent]) return;
    modeBusy.value[agent] = true;
    modeMessage.value[agent] = null;
    try {
      await agentAskQuestionSet(agent, enabled);
      modeError.value[agent] = false;
    } catch (e) {
      modeMessage.value[agent] = String(e);
      modeError.value[agent] = true;
    } finally {
      modeBusy.value[agent] = false;
      await refreshMode(agent);
    }
  }

  async function toggleStop(agent: AgentId, enabled: boolean) {
    if (modeBusy.value[agent]) return;
    modeBusy.value[agent] = true;
    modeMessage.value[agent] = null;
    try {
      await agentStopSet(agent, enabled);
      modeError.value[agent] = false;
    } catch (e) {
      modeMessage.value[agent] = String(e);
      modeError.value[agent] = true;
    } finally {
      modeBusy.value[agent] = false;
      await refreshMode(agent);
    }
  }

  // "Allow delegated runs to use AskHuman": a pure preference, no artifact is rewritten.
  async function toggleDelegated(agent: AgentId, enabled: boolean) {
    if (modeBusy.value[agent]) return;
    modeBusy.value[agent] = true;
    modeMessage.value[agent] = null;
    try {
      await agentDelegatedSet(agent, enabled);
      modeError.value[agent] = false;
    } catch (e) {
      modeMessage.value[agent] = String(e);
      modeError.value[agent] = true;
    } finally {
      modeBusy.value[agent] = false;
      await refreshMode(agent);
    }
  }

  function permissionBlockedText(reason: string): string {
    return reason === "allow_managed_hooks_only"
      ? t("settings.integration.permissionManagedOnly")
      : t("settings.integration.permissionHooksDisabled");
  }

  // 单项更新：只把某个产物（rule / hook / mcp）刷新到最新。
  async function updateArtifact(agent: AgentId, artifact: "rule" | "hook" | "mcp") {
    if (modeBusy.value[agent]) return;
    modeBusy.value[agent] = true;
    modeMessage.value[agent] = null;
    try {
      await agentModeUpdateArtifact(agent, artifact);
      modeError.value[agent] = false;
    } catch (e) {
      modeMessage.value[agent] = String(e);
      modeError.value[agent] = true;
    } finally {
      modeBusy.value[agent] = false;
      await refreshMode(agent);
      if (artifact === "hook" && supportsAgentTasks) {
        await refreshAgentTaskSettings(false);
      }
    }
  }

  // 跨所有 Agent 的「待更新」概览统计：分别数出有多少家 Rule / Hook / MCP 配置过期或缺失。
  const updateSummary = computed(() => {
    let rule = 0;
    let hook = 0;
    let mcp = 0;
    for (const a of AGENTS) {
      const m = modes.value[a.id];
      if (m.ruleNeedsUpdate) rule++;
      if (m.hookNeedsUpdate) hook++;
      if (m.mcpNeedsUpdate) mcp++;
    }
    return { rule, hook, mcp, total: rule + hook + mcp };
  });

  const updateAllBusy = ref(false);

  // 一键更新所有 Agent 的过期产物（逐家调用整模式更新，幂等）。
  async function updateAll() {
    if (updateAllBusy.value) return;
    updateAllBusy.value = true;
    try {
      for (const a of AGENTS) {
        if (!modes.value[a.id].needsUpdate) continue;
        try {
          await agentModeUpdate(a.id);
          modeError.value[a.id] = false;
        } catch (e) {
          modeMessage.value[a.id] = String(e);
          modeError.value[a.id] = true;
        }
        await refreshMode(a.id);
      }
    } finally {
      updateAllBusy.value = false;
      if (supportsAgentTasks) await refreshAgentTaskSettings(false);
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

  let promptRequest = 0;
  async function loadPrompt() {
    const request = ++promptRequest;
    const text = await getPrompt(promptVariant.value, promptAgent.value || undefined);
    if (request === promptRequest) prompt.value = text;
  }

  function setPromptVariant(v: "cli" | "mcp") {
    if (promptVariant.value === v) return;
    promptVariant.value = v;
    void loadPrompt();
  }

  function setPromptAgent(agent: AgentId | "") {
    promptAgent.value = agent;
    if (!promptSupportsCli.value) promptVariant.value = "mcp";
    if (!promptSupportsMcp.value) promptVariant.value = "cli";
    void loadPrompt();
  }

  const collabBusy = ref<Record<string, boolean>>({});
  const collabError = ref<Record<string, string | null>>({});
  const collabMessage = ref<Record<string, string>>({});
  const collabResults = ref<Record<string, CollaborationStyleApplyResult[]>>({});
  const collabDefaults = ref({ aligned: "", autonomous: "" });

  async function ensureCollabDefaults() {
    collabDefaults.value = await collaborationStyleDefaults();
  }

  function collaborationSummary(agent: AgentId) {
    const general = config.value?.general;
    if (!general) return "";
    const chosen = general.agentCollaborationStyles?.[agent]?.style;
    const value = chosen ?? general.collaborationStyle;
    const label = t(`settings.integration.collab${value[0].toUpperCase()}${value.slice(1)}`);
    return chosen ? label : t("settings.integration.collabInheritSummary", { style: label });
  }

  async function saveAgentCollaborationStyle(
    agent: AgentId | null,
    style: CollaborationStyle | null,
    text: string | null,
    retry = false,
  ) {
    const key = agent ?? "default";
    if (!config.value || collabBusy.value[key]) return;
    collabBusy.value[key] = true;
    collabError.value[key] = null;
    collabMessage.value[key] = "";
    try {
      const saved = await saveCollaborationStyle(agent, style, text, retry);
      const general = config.value.general;
      if (agent) {
        general.agentCollaborationStyles ??= {};
        general.agentCollaborationStyles[agent] = saved.general.agentCollaborationStyles?.[agent] ?? {};
      } else {
        general.collaborationStyle = saved.general.collaborationStyle;
        general.collaborationStyleCustomText = saved.general.collaborationStyleCustomText;
      }
      collabResults.value[key] = saved.results;
      // Clear a retry's earlier error wherever the default-scope save displayed it.
      for (const result of saved.results) {
        for (const [scope, results] of Object.entries(collabResults.value)) {
          if (scope === key) continue;
          collabResults.value[scope] = results.map((previous) =>
            previous.agent === result.agent ? result : previous,
          );
        }
      }
      const applied = saved.results.some((result) => result.applied);
      let messageKey = "collabSaved";
      if (agent && modes.value[agent].mode === "none") {
        messageKey = "collabSavedPending";
      } else if (applied) {
        messageKey = agent === "codex" ? "collabSavedCodex" : "collabSavedRestart";
      }
      collabMessage.value[key] = t(`settings.integration.${messageKey}`);
      await Promise.all([
        loadPrompt(),
        ...AGENTS.filter((candidate) => !agent || candidate.id === agent).map((candidate) => refreshMode(candidate.id)),
      ]);
    } catch (error) {
      collabError.value[key] = String(error);
    } finally {
      collabBusy.value[key] = false;
    }
  }

  async function retryCollaborationStyle(agent: AgentId) {
    const preference = config.value?.general.agentCollaborationStyles?.[agent];
    await saveAgentCollaborationStyle(agent, preference?.style ?? null, preference?.customText ?? null, true);
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

  async function copyPrompt() {
    try {
      await navigator.clipboard.writeText(prompt.value);
    } catch {
      /* 忽略：剪贴板不可用时静默 */
    }
    promptCopied.value = true;
    setTimeout(() => (promptCopied.value = false), 1500);
  }

  // 集成域初始化：加载提示词、可执行路径与各 Agent 集成状态。
  async function initIntegration() {
    try {
      await ensureCollabDefaults();
      await loadPrompt();
    } catch (error) {
      collabError.value.default = String(error);
    }
    try {
      mcpExePath.value = await mcpCommandPath();
    } catch {
      /* 取不到路径时保留占位符 */
    }
    await Promise.all(AGENTS.map((a) => refreshMode(a.id)));
  }

  // Which cards have their details open (and the rule that opens one when something needs the user).
  const details = useAgentDetails(AGENTS, modes, modeReadError, piVersionLoading);

  return {
    ...details,
    revealLabel,
    prompt,
    promptCopied,
    promptVariant,
    promptAgent,
    promptSupportsCli,
    promptSupportsMcp,
    setPromptAgent,
    collabBusy,
    collabError,
    collabDefaults,
    collabMessage,
    collabResults,
    collaborationSummary,
    saveAgentCollaborationStyle,
    retryCollaborationStyle,
    AGENTS,
    modes,
    integrationLoading,
    piVersionLoading,
    ensureIntegration,
    ensurePiVersion,
    ensureCodexVersion,
    modeBusy,
    modeMessage,
    modeReadError,
    modeError,
    refreshMode,
    setMode,
    togglePermission,
    toggleLifecycle,
    toggleStop,
    toggleDelegated,
    toggleAskQuestion,
    permissionBlockedText,
    updateArtifact,
    updateSummary,
    updateAllBusy,
    updateAll,
    openMenuKey,
    toggleOpenMenu,
    closeOpenMenu,
    revealFile,
    openFile,
    setPromptVariant,
    mcpExampleJson,
    mcpExampleToml,
    mcpJsonCopied,
    mcpTomlCopied,
    copyMcpExample,
    copyPrompt,
    initIntegration,
  };
}
