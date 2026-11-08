// Settings → Advanced: the Agent readiness list is a matrix (one row per Agent, one fixed column per
// condition, a check where met and a cross button where missing; an Agent that cannot start tasks is
// a grey "Unsupported" row with dashes), and the two cards that belong to a single Agent carry that
// Agent's badge in front of their title (docs/specs/settings-agents-channels-cards.md §8).
import { flushPromises, mount } from "@vue/test-utils";
import { ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { CursorLaunchStatus } from "../../lib/ipc";
import type { AgentKind, AgentTaskReadiness, AppConfig } from "../../lib/types";
import AdvancedTab from "./AdvancedTab.vue";
import type { ReadinessIssue } from "./readiness";

const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("./context", () => ({ useSettingsContext }));
// The two Agent cards read their own data when they mount.
vi.mock("../../lib/ipc", () => ({
  agentTaskCursorPlan: vi.fn(async () => "legacy"),
  permissionRulesPanel: vi.fn(async () => ({ kind: "summaries", sessions: [], globalCount: 0 })),
}));

function agent(
  kind: AgentKind,
  label: string,
  missing: ReadinessIssue[] = [],
  launchSupported = true,
): AgentTaskReadiness {
  const lacks = (what: ReadinessIssue) => missing.includes(what);
  return {
    kind,
    label,
    command: kind,
    executable: lacks("binary") ? null : `/usr/local/bin/${kind}`,
    version: null,
    binaryReady: !lacks("binary"),
    lifecycleReady: !lacks("lifecycle"),
    integrationReady: !lacks("integration"),
    integrationMode: "cli",
    ready: missing.length === 0,
    launchSupported,
    diagnostics: [],
  };
}

const AGENTS = [
  agent("claude", "Claude Code"),
  agent("codex", "Codex", ["lifecycle", "integration"]),
  agent("pi", "Pi", ["binary", "lifecycle", "integration"]),
];
// Antigravity has no New Task path: it lacks everything, and nothing the person does changes that.
const ANTIGRAVITY = agent("antigravity", "Antigravity", ["binary", "lifecycle", "integration"], false);

const CONDITIONS: ReadinessIssue[] = ["binary", "lifecycle", "integration"];
const AGENT_ROWS = ".readiness-matrix .readiness-row:not(.readiness-head)";

function mountTab(items: AgentTaskReadiness[] = AGENTS, options: { busy?: boolean } = {}) {
  const config = ref({
    general: { daemonLifecycle: "keepalive" },
    channels: { autoActivation: false, autoEndWatch: false },
    agentTasks: {
      enabled: true,
      permissionPrompt: "ask",
      cursorMax: { policy: "enforce-off", plan: "auto" },
    },
    permissions: { codexRelaxedShell: false },
  } as unknown as AppConfig);
  const api = {
    config,
    isWindows: ref(false),
    supportsAgentTasks: ref(true),
    persist: vi.fn(async () => {}),
    changeDaemonLifecycle: vi.fn(),
    toggleAgentTasks: vi.fn(),
    testAgentTaskTerminal: vi.fn(),
    taskSettingsBusy: ref(options.busy ?? false),
    taskSettingsMessage: ref(""),
    refreshAgentTaskSettings: vi.fn(),
    taskReadiness: ref(items),
    taskWorkspaces: ref([]),
    openReadinessIssue: vi.fn(),
    openWorkspacePanel: vi.fn(),
  };
  useSettingsContext.mockReturnValue(api);
  return { wrapper: mount(AdvancedTab, { global: { plugins: [i18n] } }), api };
}

describe("AdvancedTab: Agent readiness", () => {
  beforeEach(() => {
    // The Codex card teleports its session sheet into the settings window.
    document.body.innerHTML = '<div class="settings"></div>';
    i18n.global.locale.value = "en";
    useSettingsContext.mockReset();
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("shows each Agent as its badge, its name and one status tag", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    const rows = wrapper.findAll(AGENT_ROWS);
    expect(rows.map((row) => row.get(".brand-badge").attributes("data-badge"))).toEqual([
      "claude",
      "codex",
      "pi",
    ]);
    expect(rows.map((row) => row.get(".label").text())).toEqual([
      "Claude Code",
      "Codex",
      "Pi",
    ]);

    const ready = wrapper.get('[data-testid="readiness-tag-claude"]');
    expect(ready.text()).toBe("Ready");
    expect(ready.classes()).toEqual(expect.arrayContaining(["status-tag", "ok"]));
    for (const kind of ["codex", "pi"]) {
      const tag = wrapper.get(`[data-testid="readiness-tag-${kind}"]`);
      expect(tag.text(), kind).toBe("Not ready");
      expect(tag.classes(), kind).toEqual(expect.arrayContaining(["status-tag", "warn"]));
    }
  });

  it("gives every Agent the same three condition columns, under one localized header", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    const head = wrapper.get(".readiness-head");
    expect(head.findAll(".readiness-cell").map((cell) => cell.text())).toEqual([
      "CLI",
      "Lifecycle",
      "Integration",
    ]);
    // The header is decoration for the eye: each cell carries its own accessible name.
    expect(head.attributes("aria-hidden")).toBe("true");
    for (const row of wrapper.findAll(AGENT_ROWS)) {
      expect(row.findAll(".readiness-cell")).toHaveLength(CONDITIONS.length);
    }
  });

  it("shows a check where a condition is met and a cross button where it is missing, in column order", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    const cells = (kind: string) =>
      wrapper
        .findAll(`[data-testid="readiness-${kind}"] .readiness-cell`)
        .map((cell) => (cell.find(".readiness-fix").exists() ? "missing" : cell.find(".readiness-ok").exists() ? "met" : "?"));
    expect(cells("claude")).toEqual(["met", "met", "met"]);
    expect(cells("codex")).toEqual(["met", "missing", "missing"]);
    expect(cells("pi")).toEqual(["missing", "missing", "missing"]);

    // Only a missing condition is a button; the checks are not clickable.
    expect(wrapper.findAll('[data-testid="readiness-claude"] button')).toHaveLength(0);
    expect(wrapper.findAll('[data-testid="readiness-codex"] button')).toHaveLength(2);
    // No tick or cross glyph is printed as text: the marks are drawn.
    for (const row of wrapper.findAll(AGENT_ROWS)) expect(row.text()).not.toMatch(/[✓×]/);
  });

  it("names each mark for assistive technology and says what a cross does", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    expect(
      wrapper.get('[data-testid="readiness-claude"] .readiness-ok').attributes("aria-label"),
    ).toBe("Claude Code: CLI");

    const binary = wrapper.get('[data-testid="readiness-issue-pi-binary"]');
    const lifecycle = wrapper.get('[data-testid="readiness-issue-codex-lifecycle"]');
    const integration = wrapper.get('[data-testid="readiness-issue-codex-integration"]');
    expect(binary.attributes("aria-label")).toBe("Pi: CLI unavailable");
    expect(lifecycle.attributes("aria-label")).toBe("Codex: Lifecycle tracking not ready");
    expect(integration.attributes("aria-label")).toBe("Codex: Integration unavailable");
    expect(binary.attributes("title")).toBe("CLI unavailable · Open official installation guide");
    expect(lifecycle.attributes("title")).toBe(
      "Lifecycle tracking not ready · Go to the corresponding setting",
    );
    expect(integration.attributes("title")).toBe(
      "Integration unavailable · Go to the corresponding setting",
    );
  });

  it("sends each missing condition to where it is fixed", async () => {
    const { wrapper, api } = mountTab();
    await flushPromises();

    await wrapper.get('[data-testid="readiness-issue-pi-binary"]').trigger("click");
    await wrapper.get('[data-testid="readiness-issue-codex-lifecycle"]').trigger("click");
    await wrapper.get('[data-testid="readiness-issue-codex-integration"]').trigger("click");
    expect(api.openReadinessIssue.mock.calls).toEqual([
      ["pi", "binary"],
      ["codex", "lifecycle"],
      ["codex", "integration"],
    ]);
  });

  it("shows an Agent that cannot start tasks as a grey tag with dashes, not as crosses to fix", async () => {
    const { wrapper } = mountTab([...AGENTS, ANTIGRAVITY]);
    await flushPromises();

    const tag = wrapper.get('[data-testid="readiness-tag-antigravity"]');
    expect(tag.text()).toBe("Unsupported");
    // Grey, like the Channels page's "not configured": not the orange of "Not ready".
    expect(tag.classes()).toEqual(expect.arrayContaining(["status-tag", "muted"]));
    expect(tag.classes()).not.toContain("warn");

    const row = wrapper.get('[data-testid="readiness-antigravity"]');
    const cells = row.findAll(".readiness-cell");
    expect(cells).toHaveLength(CONDITIONS.length);
    expect(cells.map((cell) => cell.text())).toEqual(["–", "–", "–"]);
    // Nothing to click, and no ready-check either: no condition applies.
    expect(row.findAll("button")).toHaveLength(0);
    expect(row.find(".readiness-ok").exists()).toBe(false);
    expect(row.find(".readiness-fix").exists()).toBe(false);
  });

  it("keeps the supported rows around an unsupported one exactly as they were", async () => {
    const { wrapper } = mountTab([...AGENTS, ANTIGRAVITY]);
    await flushPromises();

    expect(wrapper.get('[data-testid="readiness-tag-claude"]').text()).toBe("Ready");
    expect(wrapper.get('[data-testid="readiness-tag-pi"]').text()).toBe("Not ready");
    expect(wrapper.findAll('[data-testid="readiness-pi"] .readiness-fix')).toHaveLength(3);
  });

  it("says what \"Unsupported\" means only when such an Agent is listed", async () => {
    const without = mountTab();
    await flushPromises();
    expect(without.wrapper.get(".readiness-hint").text()).toBe(
      i18n.global.t("settings.agentTasks.readinessHint"),
    );

    const withOne = mountTab([...AGENTS, ANTIGRAVITY]);
    await flushPromises();
    const hint = withOne.wrapper.get(".readiness-hint").text();
    expect(hint).toContain(i18n.global.t("settings.agentTasks.readinessHint"));
    expect(hint).toContain(i18n.global.t("settings.agentTasks.unsupportedHint"));
  });

  it("speaks the language for the unsupported tag too", async () => {
    const { wrapper } = mountTab([...AGENTS, ANTIGRAVITY]);
    i18n.global.locale.value = "zh";
    await flushPromises();

    expect(wrapper.get('[data-testid="readiness-tag-antigravity"]').text()).toBe("暂不支持");
  });

  it("explains under the matrix how to read it", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    expect(wrapper.get(".readiness-hint").text()).toBe(i18n.global.t("settings.agentTasks.readinessHint"));
  });

  it("follows the language and the Agents the context reports", async () => {
    const { wrapper } = mountTab();
    i18n.global.locale.value = "zh";
    await flushPromises();

    expect(wrapper.get('[data-testid="readiness-tag-claude"]').text()).toBe("已就绪");
    expect(wrapper.get('[data-testid="readiness-tag-codex"]').text()).toBe("未就绪");
    expect(wrapper.get(".readiness-head").findAll(".readiness-cell").map((cell) => cell.text())).toEqual([
      "CLI",
      "生命周期",
      "集成",
    ]);
    expect(
      wrapper.get('[data-testid="readiness-issue-pi-lifecycle"]').attributes("aria-label"),
    ).toBe("Pi: 生命周期追踪未就绪");
  });

  it("shows no matrix, only the loading state, until the first readiness arrives", async () => {
    const { wrapper } = mountTab([], { busy: true });
    await flushPromises();

    expect(wrapper.find(".readiness-matrix").exists()).toBe(false);
    expect(wrapper.find(".readiness-hint").exists()).toBe(false);
    expect(wrapper.find(".settings-section-loading").exists()).toBe(true);
  });
});

// The verdict on each row is the backend's (`startable`): the one the phone and IM act on, which also
// counts Codex's Desktop App and Cursor's Agents window. Such an Agent starts without its CLI, so the
// CLI's own columns are not crosses to fix.
describe("AdvancedTab: Agents that start in their app or window", () => {
  const CODEX_APP: AgentTaskReadiness = {
    ...agent("codex", "Codex", ["binary", "lifecycle"]),
    startable: true,
    startRoute: "desktop",
  };
  const CURSOR_WINDOW: AgentTaskReadiness = {
    ...agent("cursor", "Cursor", ["binary"]),
    startable: true,
    startRoute: "desktop",
  };

  beforeEach(() => {
    document.body.innerHTML = '<div class="settings"></div>';
    i18n.global.locale.value = "en";
    useSettingsContext.mockReset();
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("is Ready with the place it starts in, although its CLI is missing", async () => {
    const { wrapper } = mountTab([agent("claude", "Claude Code"), CODEX_APP, CURSOR_WINDOW]);
    await flushPromises();

    // A ready CLI needs no place named: the row is plainly Ready.
    expect(wrapper.get('[data-testid="readiness-tag-claude"]').text()).toBe("Ready");
    const codex = wrapper.get('[data-testid="readiness-tag-codex"]');
    expect(codex.text()).toBe("Ready · Desktop App");
    expect(codex.classes()).toEqual(expect.arrayContaining(["status-tag", "ok"]));
    expect(wrapper.get('[data-testid="readiness-tag-cursor"]').text()).toBe("Ready · Agents window");
  });

  it("shows the CLI's own conditions as dashes rather than crosses to fix", async () => {
    const { wrapper } = mountTab([CODEX_APP]);
    await flushPromises();

    const row = wrapper.get('[data-testid="readiness-codex"]');
    expect(row.findAll(".readiness-cell").map((cell) => cell.text())).toEqual(["–", "–", ""]);
    expect(row.find(".readiness-fix").exists()).toBe(false);
    const skipped = row.get('[data-testid="readiness-skip-codex-binary"]');
    expect(skipped.attributes("title")).toBe(i18n.global.t("settings.agentTasks.cliNotNeeded"));
    expect(skipped.attributes("aria-label")).toContain("Codex: CLI");
    // What is met is still a check: the integration is needed by the app route too.
    expect(row.findAll(".readiness-ok")).toHaveLength(1);
  });

  it("keeps a missing integration as a cross: the app or window route needs it as well", async () => {
    const odd: AgentTaskReadiness = {
      ...agent("cursor", "Cursor", ["binary", "integration"]),
      startable: true,
      startRoute: "desktop",
    };
    const { wrapper } = mountTab([odd]);
    await flushPromises();

    expect(wrapper.find('[data-testid="readiness-issue-cursor-integration"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="readiness-skip-cursor-integration"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="readiness-skip-cursor-binary"]').exists()).toBe(true);
  });

  it("leaves an Agent that starts through its CLI exactly as before", async () => {
    const { wrapper } = mountTab([{ ...agent("claude", "Claude Code"), startable: true, startRoute: "terminal" }]);
    await flushPromises();

    expect(wrapper.get('[data-testid="readiness-tag-claude"]').text()).toBe("Ready");
    expect(wrapper.find('[data-testid^="readiness-skip-"]').exists()).toBe(false);
    expect(wrapper.findAll('[data-testid="readiness-claude"] .readiness-ok')).toHaveLength(3);
  });

  it("takes the backend's verdict over the CLI's: a ready CLI whose routes are all unusable is Not ready", async () => {
    const { wrapper } = mountTab([{ ...agent("claude", "Claude Code"), startable: false, startRoute: null }]);
    await flushPromises();

    expect(wrapper.get('[data-testid="readiness-tag-claude"]').text()).toBe("Not ready");
  });

  it("says what the dashes mean only when such an Agent is listed", async () => {
    const without = mountTab();
    await flushPromises();
    expect(without.wrapper.get(".readiness-hint").text()).not.toContain(
      i18n.global.t("settings.agentTasks.outsideHint"),
    );

    const withOne = mountTab([...AGENTS, CODEX_APP]);
    await flushPromises();
    expect(withOne.wrapper.get(".readiness-hint").text()).toContain(
      i18n.global.t("settings.agentTasks.outsideHint"),
    );
  });

  it("speaks Chinese for the place it starts in", async () => {
    const { wrapper } = mountTab([CODEX_APP, CURSOR_WINDOW]);
    i18n.global.locale.value = "zh";
    await flushPromises();

    expect(wrapper.get('[data-testid="readiness-tag-codex"]').text()).toBe("就绪 · Desktop App");
    expect(wrapper.get('[data-testid="readiness-tag-cursor"]').text()).toBe("就绪 · Agents 窗口");
  });
});

// Starts in its app, with a launch status on it: still nothing to explain.
const CODEX_APP_FOR_BLOCKERS: AgentTaskReadiness = {
  ...agent("codex", "Codex", ["binary"]),
  startable: true,
  startRoute: "desktop",
  launch: {
    preference: "desktop",
    target: "desktop",
    desktopAvailable: true,
    terminalAvailable: false,
    desktopReason: null,
    terminalReason: "cliNotDetected",
    integrated: true,
  },
};

// An Agent with two routes that has neither: the crosses only cover the CLI's side, so the row says
// why in both routes' words (the backend's launch status).
describe("AdvancedTab: why an Agent with two routes cannot start", () => {
  const STUCK_LAUNCH: CursorLaunchStatus = {
    preference: "terminal",
    target: null,
    desktopAvailable: false,
    terminalAvailable: false,
    desktopReason: "appNotRunning",
    terminalReason: "cliNotDetected",
    integrated: true,
  };
  const STUCK_CURSOR: AgentTaskReadiness = {
    ...agent("cursor", "Cursor", ["binary", "lifecycle"]),
    startable: false,
    startRoute: null,
    launch: STUCK_LAUNCH,
  };

  beforeEach(() => {
    document.body.innerHTML = '<div class="settings"></div>';
    i18n.global.locale.value = "en";
    useSettingsContext.mockReset();
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("names what each route lacks, the preferred route first, under the Agent's row", async () => {
    const { wrapper } = mountTab([STUCK_CURSOR]);
    await flushPromises();

    const item = wrapper.get('[data-testid="readiness-cursor"]');
    const note = item.get('[data-testid="readiness-blockers-cursor"]');
    expect(note.text()).toBe(
      [
        i18n.global.t("cursorDesktop.launch.reasons.cliNotDetected"),
        i18n.global.t("cursorDesktop.launch.reasons.appNotRunning"),
      ].join(i18n.global.t("desktop.reasonSeparator")),
    );
    // Part of the same item as the row, so the two stay together.
    expect(item.find(".readiness-row").exists()).toBe(true);
    expect(item.findAll(".readiness-fix").length).toBeGreaterThan(0);
  });

  it("puts the Desktop route first when that is the preference", async () => {
    const { wrapper } = mountTab([{ ...STUCK_CURSOR, launch: { ...STUCK_LAUNCH, preference: "desktop" } }]);
    await flushPromises();

    const note = wrapper.get('[data-testid="readiness-blockers-cursor"]').text();
    expect(note.indexOf(i18n.global.t("cursorDesktop.launch.reasons.appNotRunning"))).toBeLessThan(
      note.indexOf(i18n.global.t("cursorDesktop.launch.reasons.cliNotDetected")),
    );
  });

  it("says nothing for an Agent that can start, for one with the CLI alone, or without a launch status", async () => {
    const { wrapper } = mountTab([
      CODEX_APP_FOR_BLOCKERS,
      agent("pi", "Pi", ["binary", "lifecycle", "integration"]),
      { ...STUCK_CURSOR, launch: null },
    ]);
    await flushPromises();

    expect(wrapper.find('[data-testid^="readiness-blockers-"]').exists()).toBe(false);
  });
});

describe("AdvancedTab: cards that belong to one Agent", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div class="settings"></div>';
    i18n.global.locale.value = "en";
    useSettingsContext.mockReset();
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("opens Cursor's Max card and Codex's grants card with that Agent's badge in front of the title", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    const cursorCard = wrapper.get('[data-testid="cursor-max-card"]');
    const cursorTitle = cursorCard.get(".agent-card-title");
    expect(cursorTitle.get(".brand-badge").attributes("data-badge")).toBe("cursor");
    expect(cursorTitle.get(".card-title").text()).toBe(i18n.global.t("settings.cursorMax.title"));

    const codexCard = wrapper.get(".permission-rules-card");
    const codexTitle = codexCard.get(".agent-card-title");
    expect(codexTitle.get(".brand-badge").attributes("data-badge")).toBe("codex");
    expect(codexTitle.get(".card-title").text()).toBe(i18n.global.t("settings.permissionRules.title"));
  });

  it("keeps the description outside the title row, so it spans the card's full width", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    for (const card of [
      wrapper.get('[data-testid="cursor-max-card"]'),
      wrapper.get(".permission-rules-card"),
    ]) {
      const title = card.get(".agent-card-title");
      const description = card.get(".card-desc");
      expect(title.find(".card-desc").exists()).toBe(false);
      // The description is the card's own child, next to the title row rather than inside it.
      expect(description.element.parentElement).toBe(card.element);
      expect(title.element.parentElement).toBe(card.element);
    }
  });

  it("gives no other card a badge: the daemon, delivery and task cards are not about one Agent", async () => {
    const { wrapper } = mountTab();
    await flushPromises();

    // One per readiness row, plus the two Agent cards.
    expect(wrapper.findAll(".brand-badge")).toHaveLength(AGENTS.length + 2);
    expect(wrapper.findAll(".agent-card-title")).toHaveLength(2);
  });
});
