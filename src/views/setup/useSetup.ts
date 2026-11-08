// State and actions of the setup wizard window (docs/specs/onboarding.md, docs/plans/onboarding.md §3).
//
// The wizard reuses the settings window's core (one `persist()`, the same "saved" placeholders and
// secret clearing) and its channel logic (`useChannels`: connection test, auto-detect, health), and
// only adds the step machine and the Agent-page state, so nothing here can drift from Settings.
import { computed, inject, provide, reactive, ref, watch, type InjectionKey } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  agentModeStatus,
  collaborationStyleApplyIntegrations,
  getSettings,
  setupAgentsScan,
  setupApplyAgents,
  setupChannelStatus,
  setupEnvCheck,
  setupFinish,
  setupSkip,
  setupStatus,
} from "../../lib/ipc";
import type {
  AgentId,
  CollaborationStyle,
  SettingsPayload,
  SetupAgentRow,
  SetupApplyResult,
  SetupChannelStatus,
  SetupEnvReport,
  SetupStatus,
} from "../../lib/types";
import { supportsAgentTasks } from "../../lib/platform";
import { createCore } from "../settings/context";
import { useChannels } from "../settings/useChannels";
import {
  agentsPrimary,
  isIntegrated,
  isMainRow,
  isPending,
  preselect,
  type RowPick,
} from "./agentRows";
import {
  channelDef,
  channelOps,
  channelSection,
  requiredFieldsFilled,
  type ChannelId,
} from "./channelDefs";
import { createProbes } from "./useProbe";

/** The steps the bar shows, in order. */
export const SETUP_PAGES = ["agents", "remote", "done"] as const;
export type SetupStep = (typeof SETUP_PAGES)[number];
/**
 * `tasks` is a screen inside the remote step: the offer to start Agent tasks from the phone or IM
 * (docs/specs/onboarding.md D25). It is reached from that step or from the Done page's summary
 * line, never from a URL or an event (`parsePage` only knows the steps).
 */
export type SetupPage = SetupStep | "tasks";

/** A page name from a URL or an event; anything unknown starts at the first page. */
export function parsePage(raw: string | null | undefined): SetupStep {
  return (SETUP_PAGES as readonly string[]).includes(raw ?? "") ? (raw as SetupStep) : "agents";
}

/** The step a page belongs to: the bar highlights it. */
export function stepOf(page: SetupPage): SetupStep {
  return page === "tasks" ? "remote" : page;
}

/** The per-Agent features the Done page summarises. */
export type CapabilityKey = "lifecycle" | "permission" | "stop" | "askQuestion";

export function createSetup(initialPage: SetupPage = "agents") {
  const core = createCore();
  const channels = useChannels(core);
  const { config, persist } = core;

  const page = ref<SetupPage>(initialPage);
  const status = ref<SetupStatus | null>(null);
  /** The optional checks (popup, channel round trip, Agent question); none gates finishing. */
  const probes = createProbes();

  function goto(next: SetupPage) {
    page.value = next;
    // A page change always lands on that page's list, never inside a channel's detail form.
    detail.value = null;
    // The channel round trip belongs to that form.
    probes.release("channel");
  }

  // The settings window's channel health poll only runs while its channel tab is "active"; the
  // wizard's remote page stands in for that tab.
  watch(
    page,
    (current) => {
      core.activeTab.value = current === "remote" ? "channel" : "general";
    },
    { immediate: true },
  );

  async function refreshStatus() {
    try {
      status.value = await setupStatus();
    } catch {
      /* keep the last known status */
    }
  }

  /** A Dev Instance must not touch the everyday Agent integration. */
  const readOnly = computed(
    () =>
      core.everydayIntegrationsReadOnly.value ||
      status.value?.everydayIntegrationsReadOnly === true,
  );

  // ===== Page 1: Agents =====
  const rows = ref<SetupAgentRow[]>([]);
  const rowsLoaded = ref(false);
  const rowsError = ref<string | null>(null);
  const picks = reactive<Record<string, RowPick>>({});
  const env = ref<SetupEnvReport | null>(null);

  const style = ref<CollaborationStyle>("aligned");
  const styleBaseline = ref<CollaborationStyle>("aligned");
  const applying = ref(false);
  const applyError = ref<string | null>(null);
  const applyResults = reactive<Record<string, SetupApplyResult>>({});
  const appliedOnce = ref(false);

  async function scanAgents(keepPicks = false) {
    try {
      const list = await setupAgentsScan();
      rows.value = list;
      for (const row of list) {
        if (!keepPicks || !picks[row.id]) picks[row.id] = preselect(row);
      }
      rowsError.value = null;
    } catch (e) {
      rowsError.value = String(e);
    } finally {
      rowsLoaded.value = true;
    }
  }

  async function loadEnv() {
    try {
      env.value = await setupEnvCheck();
    } catch {
      env.value = null;
    }
  }

  const mainRows = computed(() => rows.value.filter(isMainRow));
  const otherRows = computed(() => rows.value.filter((row) => !isMainRow(row)));
  const pendingRows = computed(() =>
    rows.value.filter((row) => isPending(row, picks[row.id])),
  );
  const styleChanged = computed(() => style.value !== styleBaseline.value);
  const hasChanges = computed(
    () => !readOnly.value && (pendingRows.value.length > 0 || styleChanged.value),
  );
  const agentsPrimaryKind = computed(() =>
    agentsPrimary(hasChanges.value, appliedOnce.value),
  );
  /** Something already integrated: the welcome text says what a rerun keeps. */
  const isRerun = computed(() => rows.value.some(isIntegrated));
  /** A CLI-mode Agent is integrated or about to be: it needs `AskHuman` on the login PATH. */
  const cliSelected = computed(() =>
    rows.value.some(
      (row) =>
        row.mode === "cli" || (picks[row.id]?.checked === true && picks[row.id]?.mode === "cli"),
    ),
  );
  const showPathCard = computed(
    () => env.value?.askhumanOnPath === "missing" && cliSelected.value,
  );

  /**
   * Write what the page shows: the collaboration style first (so the managed block the new
   * integrations get already carries it), then the ticked rows one by one. Results stay on the
   * page; it does not move on by itself.
   */
  async function applyAgents() {
    if (applying.value || readOnly.value || !hasChanges.value) return;
    const cfg = config.value;
    if (!cfg) return;
    applying.value = true;
    applyError.value = null;
    try {
      if (styleChanged.value) {
        cfg.general.collaborationStyle = style.value;
        await persist();
        await collaborationStyleApplyIntegrations();
        styleBaseline.value = style.value;
      }
      const selections = pendingRows.value.map((row) => ({
        agent: row.id,
        mode: picks[row.id].mode,
      }));
      if (selections.length > 0) {
        const results = await setupApplyAgents(selections);
        for (const result of results) {
          applyResults[result.agent] = result;
          // What is on disk now is the baseline; a failed row keeps its pick so it can be retried.
          if (result.ok && result.modeAfter !== "none") {
            picks[result.agent] = { checked: true, mode: result.modeAfter };
          }
        }
      }
      appliedOnce.value = true;
      await scanAgents(true);
      void refreshStatus();
    } catch (e) {
      applyError.value = String(e);
    } finally {
      applying.value = false;
    }
  }

  async function skipAgents() {
    try {
      await setupSkip("agents");
    } catch {
      /* a failed write only means the reminder stays */
    }
    await refreshStatus();
    goto("remote");
  }

  // ===== Page 2: Remote answering =====
  const channelRows = ref<SetupChannelStatus[]>([]);
  /** The channel whose detail form is open; `null` = the list. */
  const detail = ref<ChannelId | null>(null);
  const savingChannel = ref(false);
  const channelSavedId = ref<ChannelId | null>(null);
  const channelSaveError = ref<"missing" | string | null>(null);

  async function refreshChannelStatus() {
    try {
      channelRows.value = await setupChannelStatus();
    } catch {
      /* keep the last rows */
    }
  }

  const enabledChannelIds = computed(() =>
    channelRows.value.filter((row) => row.enabled).map((row) => row.id),
  );
  const hasEnabledChannel = computed(() => enabledChannelIds.value.length > 0);

  function openChannel(id: ChannelId) {
    detail.value = id;
    channelSavedId.value = null;
    channelSaveError.value = null;
    probes.release("channel");
  }

  function closeChannel() {
    detail.value = null;
    probes.release("channel");
    void refreshChannelStatus();
  }

  /** The round trip needs the channel saved and on: the check sends through the saved settings. */
  function canProbeChannel(id: ChannelId): boolean {
    const row = channelRows.value.find((candidate) => candidate.id === id);
    return row?.enabled === true && row.configured;
  }

  /** Save the credentials and turn the channel on; the same `persist()` as the settings page. */
  async function saveChannel(id: ChannelId) {
    const cfg = config.value;
    const def = channelDef(id);
    if (!cfg || !def || savingChannel.value) return;
    channelSaveError.value = null;
    channelSavedId.value = null;
    if (!requiredFieldsFilled(def, cfg, core.secretsPresent.value)) {
      channelSaveError.value = "missing";
      return;
    }
    const section = channelSection(cfg, id);
    const was = section.enabled;
    section.enabled = true;
    savingChannel.value = true;
    try {
      await persist();
      channelSavedId.value = id;
      await refreshChannelStatus();
    } catch (e) {
      section.enabled = was;
      channelSaveError.value = String(e);
    } finally {
      savingChannel.value = false;
    }
  }

  async function skipRemote() {
    try {
      await setupSkip("remote");
    } catch {
      /* the popup hint may show once more; nothing else depends on it */
    }
    await leaveRemote();
  }

  /**
   * Leave the remote step: to the Agent-task offer when it applies now, else to Done. The iOS app
   * is set up in a Settings window and an IM channel may have changed too, so what applies is
   * decided from what is saved at this moment, not from the copy this window loaded.
   */
  async function leaveRemote() {
    await adoptSavedConfig();
    await Promise.all([refreshChannelStatus(), refreshStatus()]);
    if (!rowsLoaded.value) await scanAgents(true);
    goto(showTasksOffer.value ? "tasks" : "done");
  }

  // ===== Page 3: Done =====
  // Each switch (and the Agent-task choice made on the offer screen) follows what is saved until
  // the person touches it (`null` = untouched), so a change another window makes in the meantime
  // (Settings → the iOS card's "Turn on…") is neither hidden from this page nor undone when it
  // finishes.
  const choiceA = ref<boolean | null>(null);
  const choiceB = ref<boolean | null>(null);
  const choiceTasks = ref<boolean | null>(null);
  /** A: the menu-bar icon stays and starts at login. */
  const backgroundA = computed({
    get: () => choiceA.value ?? config.value?.general.menuBarIcon === "always",
    set: (value: boolean) => {
      choiceA.value = value;
    },
  });
  /** B: the daemon stays resident (`daemonLifecycle = keepalive`). */
  const backgroundB = computed({
    get: () => choiceB.value ?? config.value?.general.daemonLifecycle === "keepalive",
    set: (value: boolean) => {
      choiceB.value = value;
    },
  });
  /**
   * The phone / IM may start Agent tasks (`agentTasks.enabled`). It is decided on the offer
   * screen; on, it pins B on.
   */
  const tasksSavedOn = computed(() => config.value?.agentTasks?.enabled === true);
  const tasksWanted = computed({
    get: () => choiceTasks.value ?? tasksSavedOn.value,
    set: (value: boolean) => {
      choiceTasks.value = value;
    },
  });
  const finishing = ref(false);
  const finishError = ref<string | null>(null);
  const capabilities = ref<CapabilityKey[]>([]);

  /** The union of the per-Agent features that are on for the integrated Agents. */
  async function loadCapabilities() {
    const found = new Set<CapabilityKey>();
    await Promise.all(
      rows.value.filter(isIntegrated).map(async (row) => {
        try {
          const mode = await agentModeStatus(row.id as AgentId);
          if (mode.lifecycle.supported && mode.lifecycle.enabled) found.add("lifecycle");
          if (mode.permission.supported && mode.permission.enabled) found.add("permission");
          if (mode.stop.supported && mode.stop.enabled) found.add("stop");
          if (mode.askQuestion.supported && mode.askQuestion.enabled) found.add("askQuestion");
        } catch {
          /* a row whose status cannot be read adds nothing */
        }
      }),
    );
    const order: CapabilityKey[] = ["lifecycle", "permission", "stop", "askQuestion"];
    capabilities.value = order.filter((key) => found.has(key));
  }

  /**
   * Asking an Agent for a question only means something once an Agent is connected, and never in
   * a Dev Instance (its Agents talk to the everyday daemon, not to this one).
   */
  const agentProbeAvailable = computed(
    () => !readOnly.value && rows.value.some(isIntegrated),
  );

  /**
   * The iOS app channel is on. It is set up in a Settings window of its own (the remote page only
   * links to it), so this is only right after `adoptSavedConfig()`.
   */
  const iosEnabled = computed(() => config.value?.channels?.ios?.enabled === true);
  /** A remote channel of any kind: an IM channel or the iOS app. */
  const hasRemoteChannel = computed(() => hasEnabledChannel.value || iosEnabled.value);

  /** Switch B is only offered once a remote channel is on. */
  const showBackgroundB = computed(() => hasRemoteChannel.value);
  /**
   * Starting Agent tasks from the phone or IM can be offered: it needs a remote channel to start
   * them from, an Agent to start, and a platform with a supported terminal.
   */
  const tasksAvailable = computed(
    () => supportsAgentTasks && hasRemoteChannel.value && rows.value.some(isIntegrated),
  );
  /** The summary line on the Done page: where the feature is on offer, and wherever it is on. */
  const showTasksRow = computed(() => tasksAvailable.value || tasksSavedOn.value);
  /** The person turned the offer down on an earlier run of the wizard. */
  const phoneTasksDeclined = computed(() => status.value?.phoneTasksDeclined === true);
  /**
   * The offer screen: on offer, and not answered yet (here, on an earlier run, or by turning the
   * feature on in Settings).
   */
  const showTasksOffer = computed(
    () =>
      tasksAvailable.value &&
      !tasksSavedOn.value &&
      choiceTasks.value === null &&
      !phoneTasksDeclined.value,
  );
  /**
   * The Agent-task feature pins B on: by the choice made on the offer screen, else by what is
   * saved.
   */
  const backgroundBLocked = computed(() => tasksWanted.value);
  /** What B's switch shows: on while pinned, otherwise the person's choice (or the saved mode). */
  const backgroundBChecked = computed({
    get: () => backgroundBLocked.value || backgroundB.value,
    set: (value: boolean) => {
      backgroundB.value = value;
    },
  });
  /** Nothing offers the feature yet: the page says where to turn it on later. */
  const showTasksTip = computed(
    () => supportsAgentTasks && rowsLoaded.value && !tasksAvailable.value && !tasksSavedOn.value,
  );

  /** "Turn on": recorded here, saved with the rest of the settings when the wizard finishes. */
  function wantTasks() {
    tasksWanted.value = true;
  }

  /**
   * "Not now": the feature stays off, and a rerun of the wizard does not ask again (Settings →
   * Advanced still has it).
   */
  async function declineTasks() {
    tasksWanted.value = false;
    // Move on first: the screen must not flip back to the question while the answer is stored.
    goto("done");
    try {
      await setupSkip("phoneTasks");
    } catch {
      /* the offer may come once more on a rerun; nothing else depends on it */
    }
    await refreshStatus();
  }

  /**
   * Replace this window's copy of the settings with what is saved now. Another window can change
   * them while this one is open (the iOS card in Settings, say); the Done page reads the copy for
   * what it offers and saves on top of it, so it must not be the older one.
   */
  async function adoptSavedConfig() {
    try {
      const payload = await getSettings();
      core.config.value = payload.config;
      core.secretsPresent.value = payload.secretsPresent;
    } catch {
      /* keep the copy loaded earlier; saving still works with it */
    }
  }

  async function closeWindow() {
    try {
      await getCurrentWebviewWindow().close();
    } catch (err) {
      console.warn("setup window close failed", err);
    }
  }

  /**
   * Done: save the background choices, record the decision (and the login item), close.
   * The page only moves a menu-bar icon that is `always` (to `active`) or turns it on; it never
   * picks `off`. Turning Agent tasks on (the offer screen's choice) also pins the daemon
   * resident; saving does the rest (Settings' `save_settings` installs the daemon's login item
   * and makes sure it is running).
   */
  async function finish() {
    if (!config.value || finishing.value) return;
    finishing.value = true;
    finishError.value = null;
    try {
      await adoptSavedConfig();
      const cfg = config.value;
      if (!cfg) return;
      if (status.value?.trayAvailable) {
        if (backgroundA.value) cfg.general.menuBarIcon = "always";
        else if (cfg.general.menuBarIcon === "always") cfg.general.menuBarIcon = "active";
      }
      // B reads the Agent-task choice, so decide both before either is written. Only what the
      // person answered on the offer screen is written; otherwise the saved value stays.
      const keepRunning = backgroundBLocked.value || backgroundB.value;
      if (choiceTasks.value !== null) cfg.agentTasks.enabled = choiceTasks.value;
      if (showBackgroundB.value) {
        cfg.general.daemonLifecycle = keepRunning ? "keepalive" : "activity";
      }
      await persist();
      await setupFinish();
      // Nothing the checks were waiting for matters once the wizard is done.
      probes.releaseAll();
      await closeWindow();
    } catch (e) {
      finishError.value = String(e);
    } finally {
      finishing.value = false;
    }
  }

  /** Fill the shared core from `get_settings` and start the slow probes without blocking paint. */
  async function load(): Promise<SettingsPayload> {
    const [payload, current] = await Promise.all([
      getSettings(),
      setupStatus().catch(() => null),
    ]);
    core.everydayIntegrationsReadOnly.value = payload.everydayIntegrationsReadOnly === true;
    core.config.value = payload.config;
    core.secretsPresent.value = payload.secretsPresent;
    status.value = current;
    const collab = payload.config.general.collaborationStyle ?? "aligned";
    style.value = collab;
    styleBaseline.value = collab;
    void scanAgents();
    void loadEnv();
    void refreshChannelStatus();
    return payload;
  }

  const ctx = {
    ...core,
    ...channels,
    page,
    goto,
    status,
    readOnly,
    refreshStatus,
    load,
    closeWindow,
    // Agents
    rows,
    rowsLoaded,
    rowsError,
    picks,
    env,
    style,
    styleBaseline,
    styleChanged,
    applying,
    applyError,
    applyResults,
    appliedOnce,
    mainRows,
    otherRows,
    pendingRows,
    hasChanges,
    agentsPrimaryKind,
    isRerun,
    showPathCard,
    scanAgents,
    applyAgents,
    skipAgents,
    // Remote
    channelRows,
    detail,
    savingChannel,
    channelSavedId,
    channelSaveError,
    enabledChannelIds,
    hasEnabledChannel,
    refreshChannelStatus,
    openChannel,
    closeChannel,
    saveChannel,
    skipRemote,
    leaveRemote,
    canProbeChannel,
    channelOpsFor: (id: ChannelId) => channelOps(id, channels),
    // Optional checks
    probes,
    agentProbeAvailable,
    // Agent tasks from the phone or IM (the offer screen, and the Done page's summary line)
    tasksWanted,
    tasksSavedOn,
    tasksAvailable,
    showTasksRow,
    showTasksOffer,
    phoneTasksDeclined,
    wantTasks,
    declineTasks,
    // Done
    backgroundA,
    backgroundBChecked,
    iosEnabled,
    hasRemoteChannel,
    showBackgroundB,
    backgroundBLocked,
    showTasksTip,
    adoptSavedConfig,
    finishing,
    finishError,
    capabilities,
    loadCapabilities,
    finish,
  };
  provide(SetupCtxKey, ctx);
  return ctx;
}

export type SetupContext = ReturnType<typeof createSetup>;

const SetupCtxKey: InjectionKey<SetupContext> = Symbol("setup-ctx");

export function useSetupContext(): SetupContext {
  const ctx = inject(SetupCtxKey);
  if (!ctx) throw new Error("setup context not provided");
  return ctx;
}
