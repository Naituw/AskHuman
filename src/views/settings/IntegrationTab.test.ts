import { mount, type VueWrapper } from "@vue/test-utils";
import { nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { AgentId, AgentModeStatus } from "../../lib/types";
import { modeStatus } from "./integration.testkit";
import IntegrationTab from "./IntegrationTab.vue";
import { revealSettingsTarget } from "./reveal";
import { useAgentDetails } from "./useAgentDetails";

const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("./context", () => ({ useSettingsContext }));
vi.mock("./CursorDesktopSettings.vue", () => ({ default: { template: '<div class="cursor-desktop-settings-stub" />' } }));

function agentDefinition(id: AgentId, hasTimeoutHook: boolean) {
  return {
    id,
    title: id,
    hasTimeoutHook,
    hasCli: id !== "grok",
    hasMcp: id !== "pi",
    instructionKind: id === "grok" ? "skill" : id === "codex" ? "hook" : "rule",
    recommended: id === "cursor" || id === "claude" || id === "pi" || id === "antigravity" ? "cli" : "mcp",
  };
}

function settingsContext(
  agent: ReturnType<typeof agentDefinition>,
  status: AgentModeStatus,
  piVersionProbing = false,
) {
  const modes = ref({ [agent.id]: status } as Record<AgentId, AgentModeStatus>);
  const modeReadError = ref<Partial<Record<AgentId, boolean>>>({});
  const piVersionLoading = ref(piVersionProbing);
  // The real fold state: the cards open and close exactly as they do in the settings window.
  const details = useAgentDetails([agent], modes, modeReadError, piVersionLoading);
  return {
    ...details,
    config: ref(null),
    revealLabel: "Reveal",
    prompt: ref(""),
    promptCopied: ref(false),
    promptVariant: ref<"cli" | "mcp">("cli"),
    collabBusy: ref(false),
    collabError: ref(""),
    changeCollaborationStyle: vi.fn(),
    saveCustomCollaborationText: vi.fn(),
    AGENTS: [agent],
    modes,
    modeReadError,
    integrationLoading: ref(false),
    piVersionLoading,
    modeBusy: ref({ [agent.id]: false }),
    modeMessage: ref<Record<string, string | null>>({ [agent.id]: null }),
    modeError: ref({ [agent.id]: false }),
    setMode: vi.fn(),
    togglePermission: vi.fn(),
    toggleLifecycle: vi.fn(),
    toggleStop: vi.fn(),
    toggleDelegated: vi.fn(),
    toggleAskQuestion: vi.fn(),
    permissionBlockedText: vi.fn(() => ""),
    updateArtifact: vi.fn(),
    updateSummary: ref({ total: 0, rule: 0, hook: 0, mcp: 0 }),
    updateAllBusy: ref(false),
    updateAll: vi.fn(),
    openMenuKey: ref(null),
    toggleOpenMenu: vi.fn(),
    closeOpenMenu: vi.fn(),
    revealFile: vi.fn(),
    openFile: vi.fn(),
    setPromptVariant: vi.fn(),
    mcpExampleJson: "",
    mcpExampleToml: "",
    mcpJsonCopied: ref(false),
    mcpTomlCopied: ref(false),
    copyMcpExample: vi.fn(),
    copyPrompt: vi.fn(),
    settingsTargetHighlight: ref(null),
  };
}

describe("IntegrationTab", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    useSettingsContext.mockReset();
  });

  function mountAgent(
    agent: ReturnType<typeof agentDefinition>,
    status: AgentModeStatus,
    extras: { integrationLoading?: boolean; piVersionLoading?: boolean } = {},
  ) {
    const context = settingsContext(agent, status, extras.piVersionLoading === true);
    if (extras.integrationLoading) context.integrationLoading.value = true;
    useSettingsContext.mockReturnValue(context);
    const wrapper = mount(IntegrationTab, {
      global: { plugins: [i18n] },
    });
    return { wrapper, context };
  }

  function rowByLabel(wrapper: VueWrapper, agent: AgentId, key: string) {
    const label = i18n.global.t(key);
    const row = wrapper
      .get(`#integration-${agent}`)
      .findAll(".agent-row")
      .find(
        (row) =>
          row.find(".label").exists() && row.find(".label").text() === label,
      );
    expect(row).toBeDefined();
    return row!;
  }

  it("shows Cursor Desktop control only inside an active Cursor integration", () => {
    const active = mountAgent(agentDefinition("cursor", true), modeStatus());
    expect(active.wrapper.find(".cursor-desktop-settings-stub").exists()).toBe(true);
    active.wrapper.unmount();
    const none = mountAgent(agentDefinition("cursor", true), modeStatus({ mode: "none" }));
    expect(none.wrapper.find(".cursor-desktop-settings-stub").exists()).toBe(false);
    none.wrapper.unmount();
  });

  it("shows default-on lifecycle drift inside an active Agent card", async () => {
    const lifecycle = {
      enabled: true,
      preferenceConfigured: false,
      installed: false,
      outdated: false,
      supported: true,
      needsUpdate: true,
      cleanupRequired: false,
    };
    const { wrapper, context } = mountAgent(
      agentDefinition("claude", true),
      modeStatus({ lifecycle }),
    );
    const row = rowByLabel(
      wrapper,
      "claude",
      "settings.integration.lifecycleTitle",
    );
    expect(row.get("input").element.checked).toBe(true);
    expect(row.find(".badge").text()).toBe(
      i18n.global.t("settings.integration.notConfigured"),
    );
    await row.get(".btn-update").trigger("click");
    expect(context.updateArtifact).toHaveBeenCalledWith("claude", "hook");
    const runtimeRow = rowByLabel(
      wrapper,
      "claude",
      "settings.integration.hookLabel",
    );
    expect(runtimeRow.find(".btn-update").exists()).toBe(false);
  });

  it("respects an explicit lifecycle opt-out without showing an update", () => {
    const lifecycle = {
      enabled: false,
      preferenceConfigured: true,
      installed: false,
      outdated: false,
      supported: true,
      needsUpdate: false,
      cleanupRequired: false,
    };
    const { wrapper } = mountAgent(
      agentDefinition("codex", false),
      modeStatus({ lifecycle }),
    );
    const row = rowByLabel(
      wrapper,
      "codex",
      "settings.integration.lifecycleTitle",
    );
    expect(row.get("input").element.checked).toBe(false);
    expect(row.find(".btn-update").exists()).toBe(false);
  });

  it("renders lifecycle tracking as the final option in an active Agent card", () => {
    const { wrapper } = mountAgent(
      agentDefinition("claude", true),
      modeStatus({
        permission: {
          supported: true,
          unsupportedReason: null,
          enabled: true,
          configured: true,
          outdated: false,
          needsUpdate: false,
          knownBlockedReason: null,
          otherHandlersDetected: false,
        },
        askQuestion: {
          supported: true,
          enabled: true,
          installed: true,
          outdated: false,
        },
      }),
    );
    const labels = wrapper
      .get("#integration-claude")
      .findAll(".agent-row .label")
      .map((label) => label.text());
    expect(labels[labels.length - 1]).toBe(
      i18n.global.t("settings.integration.lifecycleTitle"),
    );
  });

  it("prompts cleanup for legacy lifecycle in None mode", async () => {
    const lifecycle = {
      enabled: false,
      preferenceConfigured: false,
      installed: true,
      outdated: false,
      supported: true,
      needsUpdate: true,
      cleanupRequired: true,
    };
    const { wrapper, context } = mountAgent(
      agentDefinition("cursor", true),
      modeStatus({ mode: "none", lifecycle }),
    );
    const card = wrapper.get("#integration-cursor");
    expect(card.text()).toContain(
      i18n.global.t("settings.integration.lifecycleCleanupHint"),
    );
    await card.get(".lifecycle-cleanup .btn-update").trigger("click");
    expect(context.updateArtifact).toHaveBeenCalledWith("cursor", "hook");
  });

  it("hides lifecycle controls for a clean None mode", () => {
    const { wrapper } = mountAgent(
      agentDefinition("grok", false),
      modeStatus({
        mode: "none",
        lifecycle: {
          enabled: false,
          preferenceConfigured: true,
          installed: false,
          outdated: false,
          supported: true,
          needsUpdate: false,
          cleanupRequired: false,
        },
      }),
    );
    expect(wrapper.find("#lifecycle-grok").exists()).toBe(false);
  });

  it.each(["cursor", "claude"] as const)(
    "shows the installed %s CLI timeout hook without requiring a recovery hook",
    (agent) => {
      const { wrapper } = mountAgent(
        agentDefinition(agent, true),
        modeStatus({
          mode: "cli",
          timeoutHookInstalled: true,
          recoveryHookInstalled: false,
        }),
      );
      const hookRow = rowByLabel(
        wrapper,
        agent,
        "settings.integration.hookLabel",
      );

      expect(hookRow.find(".badge").text()).toBe(
        i18n.global.t("settings.integration.installed"),
      );
      expect(hookRow.find(".badge .dot").classes()).toContain("on");
    },
  );

  it("does not let a recovery hook mask a missing Cursor timeout hook", () => {
    const { wrapper } = mountAgent(
      agentDefinition("cursor", true),
      modeStatus({
        timeoutHookInstalled: false,
        recoveryHookInstalled: true,
      }),
    );
    const hookRow = rowByLabel(
      wrapper,
      "cursor",
      "settings.integration.hookLabel",
    );
    expect(hookRow.find(".badge").text()).toBe(
      i18n.global.t("settings.integration.notInstalled"),
    );
    expect(hookRow.find(".badge .dot").classes()).toContain("off");
  });

  it.each([false, true])(
    "presents the Codex protocol as its SessionStart hook when installed=%s",
    (installed) => {
      const { wrapper } = mountAgent(
        agentDefinition("codex", false),
        modeStatus({
          mode: "cli",
          rulePath: "~/.codex/hooks.json",
          ruleInstalled: installed,
          timeoutHookSupported: false,
          timeoutHookInstalled: false,
        }),
      );
      const protocolRow = rowByLabel(
        wrapper,
        "codex",
        "settings.integration.protocolLabel",
      );
      expect(protocolRow.find(".badge").text()).toBe(
        i18n.global.t(
          installed
            ? "settings.integration.installed"
            : "settings.integration.notInstalled",
        ),
      );
      expect(protocolRow.find(".badge .dot").classes()).toContain(
        installed ? "on" : "off",
      );
      const card = wrapper.get("#integration-codex");
      expect(card.text()).toContain("~/.codex/hooks.json");
      expect(card.text()).toContain(
        i18n.global.t("settings.integration.codexProtocolHint"),
      );
      // The protocol row is the hook: there is no separate Codex recovery / timeout hook row.
      const labels = card.findAll(".agent-row .label").map((label) => label.text());
      expect(labels).not.toContain(i18n.global.t("settings.integration.rulesLabel"));
      expect(labels).not.toContain(i18n.global.t("settings.integration.hookLabel"));
    },
  );

  it("routes a Codex protocol update through the rule artifact", async () => {
    const { wrapper, context } = mountAgent(
      agentDefinition("codex", false),
      modeStatus({
        mode: "mcp",
        ruleInstalled: true,
        ruleNeedsUpdate: true,
        needsUpdate: true,
      }),
    );
    const row = rowByLabel(
      wrapper,
      "codex",
      "settings.integration.protocolLabel",
    );
    await row.get(".btn-update").trigger("click");
    expect(context.updateArtifact).toHaveBeenCalledWith("codex", "rule");
  });

  it.each([false, true])(
    "binds the delegated-run switch to the preference (enabled=%s)",
    async (enabled) => {
      const { wrapper, context } = mountAgent(
        agentDefinition("codex", false),
        modeStatus({
          mode: "mcp",
          delegated: { supported: true, enabled },
        }),
      );
      const row = rowByLabel(
        wrapper,
        "codex",
        "settings.integration.delegatedTitle",
      );
      const input = row.get("input");
      expect(input.element.checked).toBe(enabled);
      expect(wrapper.get("#integration-codex").text()).toContain(
        i18n.global.t("settings.integration.delegatedHint"),
      );
      await input.setValue(!enabled);
      expect(context.toggleDelegated).toHaveBeenCalledWith("codex", !enabled);
    },
  );

  it("hides the delegated-run switch where it does not apply", () => {
    const unsupported = mountAgent(
      agentDefinition("claude", true),
      modeStatus({ delegated: { supported: false, enabled: false } }),
    );
    expect(unsupported.wrapper.text()).not.toContain(
      i18n.global.t("settings.integration.delegatedTitle"),
    );
    unsupported.wrapper.unmount();
    const none = mountAgent(
      agentDefinition("codex", false),
      modeStatus({
        mode: "none",
        delegated: { supported: true, enabled: false },
      }),
    );
    expect(none.wrapper.text()).not.toContain(
      i18n.global.t("settings.integration.delegatedTitle"),
    );
  });

  it("reports a Codex too old to receive the protocol through a hook", () => {
    const old = mountAgent(
      agentDefinition("codex", false),
      modeStatus({
        mode: "mcp",
        agentVersion: "0.140.0",
        minimumVersion: "0.144.1",
        versionSupported: false,
      }),
    );
    const text = old.wrapper.get("#integration-codex").text();
    expect(text).toContain("0.140.0");
    expect(text).toContain("0.144.1");
    old.wrapper.unmount();
    const fine = mountAgent(
      agentDefinition("codex", false),
      modeStatus({
        mode: "mcp",
        agentVersion: "0.150.0",
        minimumVersion: "0.144.1",
        versionSupported: true,
      }),
    );
    expect(fine.wrapper.get("#integration-codex").text()).not.toContain(
      i18n.global.t("settings.integration.codexVersionUnsupported", {
        version: "0.150.0",
        minimum: "0.144.1",
      }),
    );
  });

  it("uses the aggregate CLI hook update action without changing timeout readiness", async () => {
    const { wrapper, context } = mountAgent(
      agentDefinition("claude", true),
      modeStatus({
        timeoutHookInstalled: true,
        recoveryHookInstalled: false,
        hookNeedsUpdate: true,
      }),
    );
    const hookRow = rowByLabel(
      wrapper,
      "claude",
      "settings.integration.hookLabel",
    );
    expect(hookRow.find(".badge").text()).toBe(
      i18n.global.t("settings.integration.installed"),
    );
    await hookRow.get(".btn-update").trigger("click");
    expect(context.updateArtifact).toHaveBeenCalledWith("claude", "hook");
  });

  it.each(["cursor", "claude", "codex", "grok"] as const)(
    "routes %s MCP recovery drift through the MCP artifact",
    async (agent) => {
      const { wrapper, context } = mountAgent(
        agentDefinition(agent, agent === "cursor" || agent === "claude"),
        modeStatus({
          mode: "mcp",
          mcpConfigInstalled: true,
          mcpNeedsUpdate: true,
          recoveryHookInstalled: false,
        }),
      );
      const mcpRow = rowByLabel(
        wrapper,
        agent,
        "settings.integration.mcpConfigLabel",
      );
      expect(mcpRow.find(".badge").text()).toBe(
        i18n.global.t("settings.integration.installed"),
      );
      await mcpRow.get(".btn-update").trigger("click");
      expect(context.updateArtifact).toHaveBeenCalledWith(agent, "mcp");
    },
  );

  it("shows Pi as CLI-only and labels the managed runtime as an Extension", () => {
    const { wrapper } = mountAgent(
      agentDefinition("pi", true),
      modeStatus({
        mode: "cli",
        mcpSupported: false,
        runtimeArtifactKind: "extension",
        agentVersion: "0.82.0",
        minimumVersion: "0.82.0",
      }),
    );
    const card = wrapper.get("#integration-pi");
    const modeLabels = card.findAll(".seg").map((button) => button.text());
    expect(modeLabels).toContain(i18n.global.t("settings.integration.modeCli") + i18n.global.t("settings.integration.recommendedTag"));
    expect(modeLabels).not.toContain(i18n.global.t("settings.integration.modeMcp"));
    rowByLabel(wrapper, "pi", "settings.integration.extensionLabel");
    expect(card.text()).toContain(
      i18n.global.t("settings.integration.piPermissionUnsupported"),
    );
  });

  it("reports an unsupported Pi version in the integration card", () => {
    const { wrapper } = mountAgent(
      agentDefinition("pi", true),
      modeStatus({
        mode: "cli",
        mcpSupported: false,
        runtimeArtifactKind: "extension",
        agentVersion: "0.81.9",
        minimumVersion: "0.82.0",
        versionSupported: false,
      }),
    );
    expect(wrapper.get("#integration-pi").text()).toContain("0.81.9");
    expect(wrapper.get("#integration-pi").text()).toContain("0.82.0");
  });

  it("hides agent cards while integration status is loading", () => {
    const { wrapper } = mountAgent(
      agentDefinition("pi", true),
      modeStatus({ mode: "none" }),
      { integrationLoading: true },
    );
    expect(wrapper.find("#integration-pi").exists()).toBe(false);
    expect(wrapper.text()).toContain(i18n.global.t("common.loading"));
  });

  it("does not show a Pi version error while the version probe is in flight", () => {
    const { wrapper } = mountAgent(
      agentDefinition("pi", true),
      modeStatus({
        mode: "cli",
        mcpSupported: false,
        runtimeArtifactKind: "extension",
        versionSupported: false,
        minimumVersion: "0.82.0",
      }),
      { piVersionLoading: true },
    );
    const card = wrapper.get("#integration-pi");
    expect(card.text()).toContain(i18n.global.t("common.loading"));
    expect(card.text()).not.toContain(
      i18n.global.t("settings.integration.piVersionMissing", {
        minimum: "0.82.0",
      }),
    );
  });
  it("explains unsupported Antigravity capabilities without a lifecycle switch", () => {
    const { wrapper } = mountAgent(agentDefinition("antigravity", false), modeStatus({
      mode: "cli", mcpSupported: false, timeoutHookSupported: false,
      lifecycle: { supported: false, enabled: false, installed: false, needsUpdate: false, preferenceConfigured: false, outdated: false, cleanupRequired: false },
    }));
    const card = wrapper.get("#integration-antigravity");
    expect(card.text()).toContain(i18n.global.t("settings.integration.antigravityRecoveryUnsupported"));
    expect(card.text()).toContain(i18n.global.t("settings.integration.antigravityLifecycleUnsupported"));
    expect(card.text()).not.toContain(i18n.global.t("settings.integration.codexProtocolHint"));
    expect(card.find("#lifecycle-antigravity input").exists()).toBe(false);
  });

  it("offers family CLI and MCP without requiring an agy binary", () => {
    const { wrapper } = mountAgent(agentDefinition("antigravity", false), modeStatus({
      mode: "none", mcpSupported: true, agentVersion: null, versionSupported: false,
    }));
    const buttons = wrapper.findAll("#integration-antigravity .seg").map((button) => button.text());
    expect(buttons.some((text) => text.includes(i18n.global.t("settings.integration.modeCli")))).toBe(true);
    expect(buttons.some((text) => text.includes(i18n.global.t("settings.integration.modeMcp")))).toBe(true);
    expect(wrapper.text()).toContain(i18n.global.t("settings.integration.antigravityScope"));
  });

  it("does not report unreadable integration state as an uninstalled mode", () => {
    const context = settingsContext(agentDefinition("antigravity", false), modeStatus({ mode: "none" }));
    Object.assign(context, { modeReadError: ref({ antigravity: true }), refreshMode: vi.fn() });
    context.modeMessage.value.antigravity = "state unreadable";
    context.modeError.value.antigravity = true;
    useSettingsContext.mockReturnValue(context);
    const wrapper = mount(IntegrationTab, { global: { plugins: [i18n] } });
    expect(wrapper.find("#integration-antigravity .segmented").exists()).toBe(false);
    expect(wrapper.text()).toContain("state unreadable");
    expect(wrapper.text()).toContain(i18n.global.t("settings.integration.retryStatus"));
  });

  describe("card header and details", () => {
    const hidden = (wrapper: VueWrapper, agent: AgentId) =>
      (wrapper.get(`#agent-details-${agent}`).attributes("style") ?? "").includes("display: none");
    const toggleOf = (wrapper: VueWrapper, agent: AgentId) =>
      wrapper.get(`[data-testid="agent-details-toggle-${agent}"]`);
    const tagOf = (wrapper: VueWrapper, agent: AgentId, key: string) =>
      wrapper.find(`[data-testid="agent-tag-${agent}-${key}"]`);

    it("shows the mark, the name, an Integrated tag and the mode switch, with the rows folded", () => {
      const { wrapper } = mountAgent(agentDefinition("claude", true), modeStatus());
      const card = wrapper.get("#integration-claude");
      expect(card.find('.agent-head .brand-badge[data-badge="claude"]').exists()).toBe(true);
      expect(card.get(".agent-head .card-title").text()).toBe("claude");
      expect(tagOf(wrapper, "claude", "integrated").text()).toBe("Integrated");
      expect(tagOf(wrapper, "claude", "needsUpdate").exists()).toBe(false);
      expect(card.find(".agent-head .segmented").exists()).toBe(true);
      expect(toggleOf(wrapper, "claude").text()).toContain("Details");
      expect(toggleOf(wrapper, "claude").attributes("aria-expanded")).toBe("false");
      expect(hidden(wrapper, "claude")).toBe(true);
    });

    it("opens and folds the details from the disclosure", async () => {
      const { wrapper } = mountAgent(agentDefinition("claude", true), modeStatus());
      await toggleOf(wrapper, "claude").trigger("click");
      expect(toggleOf(wrapper, "claude").attributes("aria-expanded")).toBe("true");
      expect(hidden(wrapper, "claude")).toBe(false);
      await toggleOf(wrapper, "claude").trigger("click");
      expect(toggleOf(wrapper, "claude").attributes("aria-expanded")).toBe("false");
      expect(hidden(wrapper, "claude")).toBe(true);
    });

    it("offers the details as a full-width bar under the header that says what is inside", async () => {
      const { wrapper } = mountAgent(agentDefinition("claude", true), modeStatus());
      const card = wrapper.get("#integration-claude");
      const bar = toggleOf(wrapper, "claude");
      // A bar of its own below the header, not a small link tucked into it.
      expect(card.get(".agent-head").find('[data-testid="agent-details-toggle-claude"]').exists()).toBe(false);
      expect(bar.element.parentElement).toBe(card.element);
      expect(bar.classes()).toContain("agent-disclosure");
      expect(bar.get(".agent-disclosure-label").text()).toBe("Details");
      expect(bar.get(".agent-disclosure-hint").text()).toBe(
        i18n.global.t("settings.integration.detailsHint"),
      );
      expect(bar.get(".agent-disclosure-chevron").classes()).not.toContain("open");
      await bar.trigger("click");
      expect(bar.get(".agent-disclosure-chevron").classes()).toContain("open");
    });

    it("keeps the rows in the DOM while folded, so a jump can still find them", () => {
      const { wrapper } = mountAgent(agentDefinition("claude", true), modeStatus());
      expect(hidden(wrapper, "claude")).toBe(true);
      expect(wrapper.find("#lifecycle-claude").exists()).toBe(true);
      expect(wrapper.get("#agent-details-claude").findAll(".agent-row").length).toBeGreaterThan(0);
    });

    it("opens a card by itself, and tags it, when an update is waiting", () => {
      const { wrapper } = mountAgent(
        agentDefinition("claude", true),
        modeStatus({ needsUpdate: true, hookNeedsUpdate: true }),
      );
      expect(tagOf(wrapper, "claude", "needsUpdate").text()).toBe("Update available");
      expect(toggleOf(wrapper, "claude").attributes("aria-expanded")).toBe("true");
      expect(hidden(wrapper, "claude")).toBe(false);
    });

    it("keeps a card the user folded folded while the same update is still waiting", async () => {
      const { wrapper } = mountAgent(
        agentDefinition("claude", true),
        modeStatus({ needsUpdate: true, hookNeedsUpdate: true }),
      );
      await toggleOf(wrapper, "claude").trigger("click");
      expect(hidden(wrapper, "claude")).toBe(true);
      // Still tagged: the need is visible without opening the card.
      expect(tagOf(wrapper, "claude", "needsUpdate").exists()).toBe(true);
    });

    it("opens a Codex that is too old and tags the version", () => {
      const { wrapper } = mountAgent(
        agentDefinition("codex", false),
        modeStatus({
          mode: "mcp",
          agentVersion: "0.140.0",
          minimumVersion: "0.144.1",
          versionSupported: false,
        }),
      );
      expect(tagOf(wrapper, "codex", "unsupportedVersion").text()).toBe("Version unsupported");
      expect(hidden(wrapper, "codex")).toBe(false);
    });

    it("does not judge Pi's version while the probe is still running", () => {
      const { wrapper } = mountAgent(
        agentDefinition("pi", true),
        modeStatus({
          mode: "cli",
          mcpSupported: false,
          runtimeArtifactKind: "extension",
          versionSupported: false,
          minimumVersion: "0.82.0",
        }),
        { piVersionLoading: true },
      );
      expect(tagOf(wrapper, "pi", "unsupportedVersion").exists()).toBe(false);
      expect(hidden(wrapper, "pi")).toBe(true);
    });

    it("tags an Agent whose own policy keeps the hooks from running", () => {
      const { wrapper } = mountAgent(
        agentDefinition("claude", true),
        modeStatus({
          permission: {
            supported: true,
            unsupportedReason: null,
            enabled: true,
            configured: true,
            outdated: false,
            needsUpdate: false,
            knownBlockedReason: "managed_hooks_disabled",
            otherHandlersDetected: false,
          },
        }),
      );
      expect(tagOf(wrapper, "claude", "blocked").text()).toBe("Hooks blocked");
    });

    it("gives a not-integrated Agent a bare header: no tag, no details", () => {
      const { wrapper } = mountAgent(agentDefinition("cursor", true), modeStatus({ mode: "none" }));
      const card = wrapper.get("#integration-cursor");
      expect(card.findAll(".status-tag")).toHaveLength(0);
      expect(card.find('[data-testid="agent-details-toggle-cursor"]').exists()).toBe(false);
      expect(card.find("#agent-details-cursor").exists()).toBe(false);
      expect(card.find(".agent-head .segmented").exists()).toBe(true);
    });

    it("tags the leftovers of an earlier integration on a not-integrated Agent", () => {
      const lifecycle = {
        enabled: false,
        preferenceConfigured: false,
        installed: true,
        outdated: false,
        supported: true,
        needsUpdate: true,
        cleanupRequired: true,
      };
      const { wrapper } = mountAgent(
        agentDefinition("cursor", true),
        modeStatus({ mode: "none", lifecycle }),
      );
      expect(tagOf(wrapper, "cursor", "cleanup").text()).toBe("Needs cleanup");
      expect(tagOf(wrapper, "cursor", "integrated").exists()).toBe(false);
    });

    it("tags an unreadable status and shows no details", () => {
      const context = settingsContext(agentDefinition("antigravity", false), modeStatus({ mode: "none" }));
      context.modeReadError.value = { antigravity: true };
      Object.assign(context, { refreshMode: vi.fn() });
      useSettingsContext.mockReturnValue(context);
      const wrapper = mount(IntegrationTab, { global: { plugins: [i18n] } });
      expect(tagOf(wrapper, "antigravity", "unreadable").text()).toBe("Can't read status");
      expect(wrapper.find("#agent-details-antigravity").exists()).toBe(false);
    });

    it("opens the card when a jump to one of its rows announces itself", async () => {
      const { wrapper } = mountAgent(agentDefinition("claude", true), modeStatus(), {});
      expect(hidden(wrapper, "claude")).toBe(true);
      // `attachTo`-less on purpose: the event bubbles through the component's own DOM.
      await revealSettingsTarget(wrapper.get("#lifecycle-claude").element);
      expect(hidden(wrapper, "claude")).toBe(false);
      expect(toggleOf(wrapper, "claude").attributes("aria-expanded")).toBe("true");
    });

    it("lets a jump to a card that has nothing folded pass without effect", async () => {
      const { wrapper } = mountAgent(agentDefinition("cursor", true), modeStatus({ mode: "none" }));
      await expect(revealSettingsTarget(wrapper.get("#integration-cursor").element)).resolves.toBeUndefined();
      expect(wrapper.find("#agent-details-cursor").exists()).toBe(false);
    });

    it("puts a status message under the header while folded and at the end of the details when open", async () => {
      const { wrapper, context } = mountAgent(agentDefinition("claude", true), modeStatus());
      context.modeMessage.value.claude = "Saved";
      await nextTick();
      const card = wrapper.get("#integration-claude");
      expect(card.text().match(/Saved/g)).toHaveLength(1);
      expect(card.get("#agent-details-claude").text()).not.toContain("Saved");
      await toggleOf(wrapper, "claude").trigger("click");
      expect(card.text().match(/Saved/g)).toHaveLength(1);
      expect(card.get("#agent-details-claude").text()).toContain("Saved");
    });
  });

});
