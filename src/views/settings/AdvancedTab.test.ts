// Settings → Advanced: the Agent readiness list is a matrix (one row per Agent, one fixed column per
// condition, a check where met and a cross button where missing), and the two cards that belong to a
// single Agent carry that Agent's badge in front of their title
// (docs/specs/settings-agents-channels-cards.md §8).
import { flushPromises, mount } from "@vue/test-utils";
import { ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
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

function agent(kind: AgentKind, label: string, missing: ReadinessIssue[] = []): AgentTaskReadiness {
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
    launchSupported: true,
    diagnostics: [],
  };
}

const AGENTS = [
  agent("claude", "Claude Code"),
  agent("codex", "Codex", ["lifecycle", "integration"]),
  agent("antigravity", "Antigravity", ["binary", "lifecycle", "integration"]),
];

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
      "antigravity",
    ]);
    expect(rows.map((row) => row.get(".label").text())).toEqual([
      "Claude Code",
      "Codex",
      "Antigravity",
    ]);

    const ready = wrapper.get('[data-testid="readiness-tag-claude"]');
    expect(ready.text()).toBe("Ready");
    expect(ready.classes()).toEqual(expect.arrayContaining(["status-tag", "ok"]));
    for (const kind of ["codex", "antigravity"]) {
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
    expect(cells("antigravity")).toEqual(["missing", "missing", "missing"]);

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

    const binary = wrapper.get('[data-testid="readiness-issue-antigravity-binary"]');
    const lifecycle = wrapper.get('[data-testid="readiness-issue-codex-lifecycle"]');
    const integration = wrapper.get('[data-testid="readiness-issue-codex-integration"]');
    expect(binary.attributes("aria-label")).toBe("Antigravity: CLI unavailable");
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

    await wrapper.get('[data-testid="readiness-issue-antigravity-binary"]').trigger("click");
    await wrapper.get('[data-testid="readiness-issue-codex-lifecycle"]').trigger("click");
    await wrapper.get('[data-testid="readiness-issue-codex-integration"]').trigger("click");
    expect(api.openReadinessIssue.mock.calls).toEqual([
      ["antigravity", "binary"],
      ["codex", "lifecycle"],
      ["codex", "integration"],
    ]);
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
      wrapper.get('[data-testid="readiness-issue-antigravity-lifecycle"]').attributes("aria-label"),
    ).toBe("Antigravity: 生命周期追踪未就绪");
  });

  it("shows no matrix, only the loading state, until the first readiness arrives", async () => {
    const { wrapper } = mountTab([], { busy: true });
    await flushPromises();

    expect(wrapper.find(".readiness-matrix").exists()).toBe(false);
    expect(wrapper.find(".readiness-hint").exists()).toBe(false);
    expect(wrapper.find(".settings-section-loading").exists()).toBe(true);
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
