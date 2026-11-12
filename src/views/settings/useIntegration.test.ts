import { defineComponent, ref } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import * as ipc from "../../lib/ipc";
import type { CollaborationStyleSaved } from "../../lib/types";
import { settings } from "../setup/setup.testkit";
import type { SettingsCore } from "./context";
import { modeStatus } from "./integration.testkit";
import { useIntegration } from "./useIntegration";

vi.mock("../../lib/ipc");

describe("per-Agent collaboration settings", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(ipc.agentModeStatus).mockResolvedValue(modeStatus());
    vi.mocked(ipc.getPrompt).mockResolvedValue("saved prompt");
  });

  function harness() {
    const payload = settings();
    const core: SettingsCore = {
      config: ref(payload.config), activeTab: ref("integration"),
      secretsPresent: ref(payload.secretsPresent), persist: vi.fn(),
    };
    let integration!: ReturnType<typeof useIntegration>;
    const wrapper = mount(defineComponent({
      setup() {
        integration = useIntegration(core, {
          refreshAgentTaskSettings: vi.fn(), ensurePiReadiness: vi.fn(),
          ensureCodexReadiness: vi.fn(), taskReadiness: ref([]),
        });
        return () => null;
      },
    }), { global: { plugins: [i18n] } });
    return { core, integration, wrapper };
  }

  it("merges out-of-order saves by scope without dropping a sibling preference", async () => {
    const { core, integration, wrapper } = harness();
    let resolveCodex!: (value: CollaborationStyleSaved) => void;
    let resolveCursor!: (value: CollaborationStyleSaved) => void;
    vi.mocked(ipc.saveCollaborationStyle).mockImplementation((agent) => new Promise((resolve) => {
      if (agent === "codex") resolveCodex = resolve;
      else resolveCursor = resolve;
    }));
    const codex = integration.saveAgentCollaborationStyle("codex", "custom", "codex body");
    const cursor = integration.saveAgentCollaborationStyle("cursor", "autonomous", null);
    resolveCursor({ general: { ...settings().config.general, agentCollaborationStyles: { cursor: { style: "autonomous" } } }, results: [] });
    await cursor;
    resolveCodex({ general: { ...settings().config.general, agentCollaborationStyles: { codex: { style: "custom", customText: "codex body" } } }, results: [] });
    await codex;
    expect(core.config.value?.general.agentCollaborationStyles).toEqual({
      codex: { style: "custom", customText: "codex body" }, cursor: { style: "autonomous" },
    });
    expect(core.persist).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("keeps saved configuration after an application failure and retries that Agent", async () => {
    const { core, integration, wrapper } = harness();
    const general = { ...settings().config.general, agentCollaborationStyles: { codex: { style: "custom" as const, customText: "own text" } } };
    vi.mocked(ipc.saveCollaborationStyle)
      .mockResolvedValueOnce({ general, results: [{ agent: "codex", applied: false, error: "write failed" }] })
      .mockResolvedValueOnce({ general, results: [{ agent: "codex", applied: true, error: null }] });
    await integration.saveAgentCollaborationStyle("codex", "custom", "own text");
    expect(core.config.value?.general.agentCollaborationStyles?.codex?.customText).toBe("own text");
    expect(integration.collabResults.value.codex[0].error).toBe("write failed");
    await integration.retryCollaborationStyle("codex");
    expect(ipc.saveCollaborationStyle).toHaveBeenLastCalledWith("codex", "custom", "own text", true);
    expect(integration.collabResults.value.codex[0].error).toBeNull();
    wrapper.unmount();
  });

  it("uses supported transports and the selected Agent in the manual preview", async () => {
    const { integration, wrapper } = harness();
    integration.setPromptAgent("grok");
    await flushPromises();
    expect(integration.promptVariant.value).toBe("mcp");
    expect(ipc.getPrompt).toHaveBeenLastCalledWith("mcp", "grok");
    integration.setPromptAgent("pi");
    await flushPromises();
    expect(integration.promptVariant.value).toBe("cli");
    expect(ipc.getPrompt).toHaveBeenLastCalledWith("cli", "pi");
    wrapper.unmount();
  });
});
