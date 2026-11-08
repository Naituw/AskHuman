// useAgentTasks: where a readiness link goes (docs/specs/settings-agents-channels-cards.md §8).
import { mount } from "@vue/test-utils";
import { defineComponent, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { AppConfig } from "../../lib/types";
import type { SettingsCore } from "./context";
import { AGENT_INSTALL_DOCS, useAgentTasks } from "./useAgentTasks";
import { SETTINGS_REVEAL_EVENT } from "./reveal";

const ipc = vi.hoisted(() => ({
  agentTaskReadiness: vi.fn(),
  agentTaskTestTerminal: vi.fn(),
  agentTaskWorkspaceAdd: vi.fn(),
  agentTaskWorkspaceForget: vi.fn(),
  agentTaskWorkspaceHide: vi.fn(),
  agentTaskWorkspacePin: vi.fn(),
  agentTaskWorkspaces: vi.fn(),
  openPath: vi.fn(async () => {}),
}));

vi.mock("../../lib/ipc", () => ipc);
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

function mountComposable() {
  const activeTab = ref("advanced");
  const core = {
    config: ref<AppConfig | null>(null),
    activeTab,
    secretsPresent: ref({}),
    persist: vi.fn(async () => {}),
  } as unknown as SettingsCore;
  let api!: ReturnType<typeof useAgentTasks>;
  const wrapper = mount(
    defineComponent({
      setup() {
        api = useAgentTasks(core);
        return () => h("div");
      },
    }),
    { global: { plugins: [i18n] } },
  );
  return { wrapper, api, activeTab };
}

/** Put an element with `id` in the document; it records the scroll and the reveal request it gets. */
function addTarget(id: string) {
  const el = document.createElement("div");
  el.id = id;
  const scrollIntoView = vi.fn();
  el.scrollIntoView = scrollIntoView;
  const revealed = vi.fn();
  el.addEventListener(SETTINGS_REVEAL_EVENT, revealed);
  document.body.appendChild(el);
  return { el, scrollIntoView, revealed };
}

describe("useAgentTasks: readiness links", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    ipc.openPath.mockClear();
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.useRealTimers();
  });

  it("opens the Agent's official install guide for a missing CLI, without leaving the tab", async () => {
    const { api, activeTab } = mountComposable();

    await api.openReadinessIssue("claude", "binary");

    expect(ipc.openPath).toHaveBeenCalledWith(AGENT_INSTALL_DOCS.claude);
    expect(activeTab.value).toBe("advanced");
  });

  it("goes to the lifecycle row of an integrated Agent", async () => {
    const { api, activeTab } = mountComposable();
    const row = addTarget("lifecycle-claude");
    const card = addTarget("integration-claude");

    await api.openReadinessIssue("claude", "lifecycle");

    expect(activeTab.value).toBe("integration");
    expect(row.revealed).toHaveBeenCalledTimes(1);
    expect(row.scrollIntoView).toHaveBeenCalledTimes(1);
    expect(api.settingsTargetHighlight.value).toBe("lifecycle-claude");
    expect(card.scrollIntoView).not.toHaveBeenCalled();
  });

  it("falls back to the Agent's card when it has no lifecycle row, instead of doing nothing", async () => {
    const { api, activeTab } = mountComposable();
    // A card that is not integrated has no lifecycle row; only the card itself is there.
    const card = addTarget("integration-codex");

    await api.openReadinessIssue("codex", "lifecycle");

    expect(activeTab.value).toBe("integration");
    expect(card.revealed).toHaveBeenCalledTimes(1);
    expect(card.scrollIntoView).toHaveBeenCalledTimes(1);
    expect(api.settingsTargetHighlight.value).toBe("integration-codex");
  });

  it("goes to the Agent's card for an unavailable integration", async () => {
    const { api } = mountComposable();
    const card = addTarget("integration-grok");

    await api.openReadinessIssue("grok", "integration");

    expect(card.scrollIntoView).toHaveBeenCalledTimes(1);
    expect(api.settingsTargetHighlight.value).toBe("integration-grok");
  });

  it("keeps a missing target without a fallback as it was: nothing to scroll to", async () => {
    const { api } = mountComposable();

    await expect(api.gotoSettingsTarget("lifecycle-nobody")).resolves.toBeUndefined();

    expect(api.settingsTargetHighlight.value).toBe("lifecycle-nobody");
  });

  it("clears the highlight a moment after the jump", async () => {
    vi.useFakeTimers();
    const { api } = mountComposable();
    addTarget("integration-pi");

    await api.gotoSettingsTarget("lifecycle-pi", "integration-pi");
    await nextTick();
    expect(api.settingsTargetHighlight.value).toBe("integration-pi");

    vi.advanceTimersByTime(2300);
    expect(api.settingsTargetHighlight.value).toBe("");
  });
});
