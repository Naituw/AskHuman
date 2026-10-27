import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import TodoSyncCard from "./TodoSyncCard.vue";

const ipc = vi.hoisted(() => ({
  todoSyncStatus: vi.fn(),
  todoSyncEnable: vi.fn(),
  todoSyncDisable: vi.fn(),
  todoSyncNow: vi.fn(),
  todoSyncClearCloud: vi.fn(),
  todoSyncResume: vi.fn(),
  todoSyncRestartFromLocal: vi.fn(),
  todoSyncRepair: vi.fn(),
  todoSyncOpenLoginItems: vi.fn(async () => {}),
}));
const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("../../lib/ipc", () => ipc);
vi.mock("./context", () => ({ useSettingsContext }));

function status(patch: Record<string, unknown> = {}) {
  const now = Date.now();
  return {
    supported: true,
    enabled: true,
    syncing: false,
    deviceName: "Studio",
    paused: null,
    lastPullOkMs: now - 40_000,
    lastPushOkMs: now - 100_000,
    unsynced: 0,
    todos: 12,
    history: 14,
    devices: [
      { name: "Studio", lastSyncMs: now - 40_000, thisMac: true },
      { name: "MacBook Pro", lastSyncMs: now - 6 * 60_000, thisMac: false },
    ],
    helper: { state: "running", version: "1", environment: "production", problem: null, installed: true },
    lastTick: null,
    newerState: false,
    ...patch,
  };
}

describe("TodoSyncCard", () => {
  const persist = vi.fn(async () => {});
  let config: ReturnType<typeof makeConfig>;

  function makeConfig() {
    return ref({ todoSync: reactive({ deviceName: "", pollMinutes: 10 }) });
  }

  beforeEach(() => {
    document.body.innerHTML = '<div class="settings"><div id="mount"></div></div>';
    i18n.global.locale.value = "en";
    Object.values(ipc).forEach((fn) => "mockReset" in fn && fn.mockReset());
    ipc.todoSyncOpenLoginItems.mockResolvedValue(undefined);
    persist.mockClear();
    config = makeConfig();
    useSettingsContext.mockReturnValue({ config, persist });
    ipc.todoSyncStatus.mockResolvedValue(status());
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.useRealTimers();
  });

  function mountCard() {
    return mount(TodoSyncCard, { attachTo: "#mount", global: { plugins: [i18n] } });
  }

  it("says that sync does not exist where it does not, and offers nothing", async () => {
    ipc.todoSyncStatus.mockResolvedValue(status({ supported: false, enabled: false }));
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.text()).toContain("macOS only");
    expect(wrapper.find(".segmented").exists()).toBe(false);
    wrapper.unmount();
  });

  it("shows the totals, this Mac's name and the Macs taking part", async () => {
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.get(".ts-status").text()).toContain("12 todos · 14 history · 40s ago");
    expect(wrapper.get(".ts-name").attributes("placeholder")).toBe("Studio");
    const devices = wrapper.findAll(".ts-device");
    expect(devices).toHaveLength(2);
    expect(devices[0].text()).toContain("this Mac");
    expect(devices[1].text()).toContain("MacBook Pro");
    expect(wrapper.get(".ts-helper").text()).toBe("running · v1");
    wrapper.unmount();
  });

  it("turns sync on from the storage switch while it is off, and shows the result", async () => {
    ipc.todoSyncStatus.mockResolvedValue(status({ enabled: false }));
    ipc.todoSyncEnable.mockResolvedValue(status({ lastPullOkMs: 0, lastPushOkMs: 0 }));
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.find(".ts-device").exists()).toBe(false);
    const [local, icloud] = wrapper.findAll(".segmented .seg");
    expect(local.classes()).toContain("active");
    await icloud.trigger("click");
    await flushPromises();
    expect(ipc.todoSyncEnable).toHaveBeenCalledTimes(1);
    expect(wrapper.findAll(".segmented .seg")[1].classes()).toContain("active");
    expect(wrapper.get(".ts-status").text()).toContain("The first sync is in progress");
    wrapper.unmount();
  });

  it("stops syncing without touching anything else", async () => {
    ipc.todoSyncDisable.mockResolvedValue(status({ enabled: false }));
    const wrapper = mountCard();
    await flushPromises();
    await wrapper.findAll(".segmented .seg")[0].trigger("click");
    await flushPromises();
    expect(ipc.todoSyncDisable).toHaveBeenCalledTimes(1);
    expect(ipc.todoSyncClearCloud).not.toHaveBeenCalled();
    expect(wrapper.find(".ts-device").exists()).toBe(false);
    wrapper.unmount();
  });

  it("syncs now on request", async () => {
    ipc.todoSyncNow.mockResolvedValue(status({ todos: 13 }));
    const wrapper = mountCard();
    await flushPromises();
    const syncNow = wrapper.findAll(".ts-actions .btn").find((b) => b.text() === "Sync now")!;
    await syncNow.trigger("click");
    await flushPromises();
    expect(ipc.todoSyncNow).toHaveBeenCalledTimes(1);
    expect(wrapper.get(".ts-status").text()).toContain("13 todos");
    wrapper.unmount();
  });

  it("asks before clearing the cloud and says every Mac is affected", async () => {
    ipc.todoSyncClearCloud.mockResolvedValue(status({ paused: "zoneMissing" }));
    const wrapper = mountCard();
    await flushPromises();
    await wrapper.get(".ts-danger-link").trigger("click");
    expect(ipc.todoSyncClearCloud).not.toHaveBeenCalled();
    const dialog = wrapper.get(".confirm-dialog");
    expect(dialog.text()).toContain("All Macs pause syncing");
    // Cancelling changes nothing.
    await dialog.findAll(".btn")[0].trigger("click");
    expect(wrapper.find(".confirm-dialog").exists()).toBe(false);
    expect(ipc.todoSyncClearCloud).not.toHaveBeenCalled();

    await wrapper.get(".ts-danger-link").trigger("click");
    await wrapper.get(".confirm-dialog .btn-primary").trigger("click");
    await flushPromises();
    expect(ipc.todoSyncClearCloud).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("was cleared");
    wrapper.unmount();
  });

  it("lets the user decide after an iCloud account change: start over from this Mac's todos", async () => {
    ipc.todoSyncStatus.mockResolvedValue(status({ paused: "accountChanged" }));
    ipc.todoSyncRestartFromLocal.mockResolvedValue(status());
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.text()).toContain("iCloud account on this Mac changed");
    await wrapper.get(".ts-actions .btn-primary").trigger("click");
    await flushPromises();
    expect(ipc.todoSyncRestartFromLocal).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).not.toContain("iCloud account on this Mac changed");
    wrapper.unmount();
  });

  it("repairs the helper and explains the approval step when macOS wants one", async () => {
    ipc.todoSyncStatus.mockResolvedValue(
      status({
        helper: { state: "notRunning", version: null, environment: null, problem: null, installed: true },
      }),
    );
    ipc.todoSyncRepair.mockResolvedValue({ ok: false, needsApproval: true, message: "requires approval" });
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.get(".ts-helper").classes()).toContain("bad");
    await wrapper.get(".ts-inline-btn").trigger("click");
    await flushPromises();
    expect(ipc.todoSyncRepair).toHaveBeenCalledTimes(1);
    expect(wrapper.get(".ts-approval").text()).toContain("Login Items");
    await wrapper.get(".ts-approval .btn").trigger("click");
    expect(ipc.todoSyncOpenLoginItems).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("reports a failed action instead of swallowing it", async () => {
    ipc.todoSyncNow.mockRejectedValue("sync failed: boom");
    const wrapper = mountCard();
    await flushPromises();
    await wrapper.findAll(".ts-actions .btn").find((b) => b.text() === "Sync now")!.trigger("click");
    await flushPromises();
    expect(wrapper.get(".result.err").text()).toContain("boom");
    wrapper.unmount();
  });

  it("persists the device name and asks for a sync so the other Macs see it", async () => {
    ipc.todoSyncNow.mockResolvedValue(status({ deviceName: "Desk" }));
    const wrapper = mountCard();
    await flushPromises();
    const input = wrapper.get(".ts-name");
    // `setValue` fires `input` and then `change`, like a user leaving the field.
    await input.setValue("Desk");
    await flushPromises();
    expect(config.value.todoSync.deviceName).toBe("Desk");
    expect(persist).toHaveBeenCalledTimes(1);
    expect(ipc.todoSyncNow).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });
});
