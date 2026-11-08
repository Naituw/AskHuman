import { mount } from "@vue/test-utils";
import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import SettingsOverlays from "./SettingsOverlays.vue";

const useSettingsContext = vi.hoisted(() => vi.fn());
vi.mock("./context", () => ({ useSettingsContext }));

type ImChannel = "telegram" | "slack" | "dingding" | "feishu";

/** The context the overlays read, with the given IM channels switched on. */
function provide(on: ImChannel[] = []) {
  const confirmEnable = vi.fn();
  const enabled = (id: ImChannel) => ({ enabled: on.includes(id) });
  useSettingsContext.mockReturnValue({
    config: ref({
      channels: {
        telegram: enabled("telegram"),
        slack: enabled("slack"),
        dingding: enabled("dingding"),
        feishu: enabled("feishu"),
      },
    }),
    agentTasksConfirmOpen: ref(true),
    confirmEnableAgentTasks: confirmEnable,
    workspacePanelOpen: ref(false),
    closeWorkspacePanel: vi.fn(),
    taskSettingsBusy: ref(false),
    taskSettingsMessage: ref(""),
    taskWorkspaces: ref([]),
    pickTaskWorkspace: vi.fn(),
    mutateTaskWorkspace: vi.fn(),
    toggleWorkspaceMenu: vi.fn(),
    workspaceMenuPath: ref(null),
    workspaceAgents: vi.fn(),
    workspaceLastUsed: vi.fn(),
  });
  return confirmEnable;
}

const mountOverlays = () => mount(SettingsOverlays, { global: { plugins: [i18n] } });

describe("the Agent-task confirmation", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });

  it("names the feature after the phone as well as IM", () => {
    provide();
    const wrapper = mountOverlays();
    expect(wrapper.get("h2").text()).toBe('Enable "Create Agent tasks from phone or IM"?');
  });

  it("lists the resident daemon and the resources it uses, and nothing about an IM bot without one", () => {
    provide();
    const items = mountOverlays()
      .findAll(".confirm-list li")
      .map((li) => li.text());
    expect(items).toHaveLength(2);
    expect(items[0]).toContain("start-at-login item");
    expect(items.join(" ")).not.toContain("IM bot");
  });

  it.each<ImChannel>(["telegram", "slack", "dingding", "feishu"])(
    "adds the note about holding the IM bot channel when %s is on",
    (channel) => {
      provide([channel]);
      const items = mountOverlays()
        .findAll(".confirm-list li")
        .map((li) => li.text());
      expect(items).toHaveLength(3);
      expect(items[2]).toContain("IM bot channel");
    },
  );

  it("turns the feature on only from its own button", async () => {
    const confirmEnable = provide();
    const wrapper = mountOverlays();
    expect(confirmEnable).not.toHaveBeenCalled();
    await wrapper.get(".confirm-actions .btn-primary").trigger("click");
    expect(confirmEnable).toHaveBeenCalledTimes(1);
  });

  it("speaks Chinese when the interface is Chinese", () => {
    i18n.global.locale.value = "zh";
    provide();
    expect(mountOverlays().get("h2").text()).toBe("开启「从手机 / IM 创建 Agent 任务」？");
  });
});
