// The one copy of the setup status the Settings window reads (docs/specs/onboarding.md D13, D15).
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { defineComponent, h } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SetupStatus } from "../../lib/types";
import { status } from "../setup/setup.testkit";
import { useSetupStatus } from "./useSetupStatus";

const ipc = vi.hoisted(() => ({
  setupStatus: vi.fn(),
  setupSkip: vi.fn(),
  setupLoginItemEnable: vi.fn(),
}));
const events = vi.hoisted(() => ({
  handlers: new Map<string, () => void>(),
  unlistened: [] as string[],
}));

vi.mock("../../lib/ipc", () => ipc);
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: () => void) => {
    events.handlers.set(name, handler);
    return () => {
      events.handlers.delete(name);
      events.unlistened.push(name);
    };
  }),
}));

describe("useSetupStatus", () => {
  let current: SetupStatus;
  let api!: ReturnType<typeof useSetupStatus>;
  let wrapper: VueWrapper;

  async function mountIt() {
    wrapper = mount(
      defineComponent({
        setup() {
          api = useSetupStatus();
          return () => h("div");
        },
      }),
    );
    await flushPromises();
  }

  beforeEach(() => {
    vi.resetAllMocks();
    events.handlers.clear();
    events.unlistened.length = 0;
    current = status({ agentsDecided: false, loginItemInstalled: false });
    ipc.setupStatus.mockImplementation(async () => ({ ...current }));
    ipc.setupSkip.mockImplementation(async () => {
      current = { ...current, agentsDecided: true };
    });
    ipc.setupLoginItemEnable.mockImplementation(async () => {
      current = { ...current, loginItemInstalled: true };
    });
  });

  afterEach(() => wrapper?.unmount());

  it("loads the status when the window mounts", async () => {
    await mountIt();
    expect(api.setupState.value).toEqual(current);
  });

  it("refreshes when the backend says something changed", async () => {
    await mountIt();
    current = { ...current, agentsDecided: true };
    events.handlers.get("setup-status-changed")!();
    await flushPromises();
    expect(api.setupState.value?.agentsDecided).toBe(true);
  });

  it("refreshes when the window gets focus, so a change made elsewhere clears the hints", async () => {
    await mountIt();
    current = { ...current, loginItemInstalled: true };
    window.dispatchEvent(new Event("focus"));
    await flushPromises();
    expect(api.setupState.value?.loginItemInstalled).toBe(true);
  });

  it("keeps the last known status when a refresh fails", async () => {
    await mountIt();
    ipc.setupStatus.mockRejectedValue(new Error("daemon is restarting"));
    await api.refreshSetupStatus();
    expect(api.setupState.value).toEqual(current);
  });

  it("stops listening when the window closes", async () => {
    await mountIt();
    wrapper.unmount();
    expect(events.unlistened).toContain("setup-status-changed");
    ipc.setupStatus.mockClear();
    window.dispatchEvent(new Event("focus"));
    await flushPromises();
    expect(ipc.setupStatus).not.toHaveBeenCalled();
  });

  it("closing the banner skips the Agent step and shows the new status", async () => {
    await mountIt();
    await api.dismissSetupBanner();
    expect(ipc.setupSkip).toHaveBeenCalledWith("agents");
    expect(api.setupState.value?.agentsDecided).toBe(true);
  });

  it("enabling the login item refreshes the status, which makes the prompt go away", async () => {
    await mountIt();
    const pending = api.enableLoginItem();
    expect(api.loginItemBusy.value).toBe(true);
    await pending;
    expect(api.loginItemBusy.value).toBe(false);
    expect(api.loginItemError.value).toBeNull();
    expect(api.setupState.value?.loginItemInstalled).toBe(true);
  });

  it("reports a login item that could not be written, and clears the error on the next try", async () => {
    await mountIt();
    ipc.setupLoginItemEnable.mockRejectedValueOnce("Operation not permitted");
    await api.enableLoginItem();
    expect(api.loginItemError.value).toBe("Operation not permitted");
    expect(api.loginItemBusy.value).toBe(false);
    expect(api.setupState.value?.loginItemInstalled).toBe(false);

    await api.enableLoginItem();
    expect(api.loginItemError.value).toBeNull();
    expect(api.setupState.value?.loginItemInstalled).toBe(true);
  });

  it("ignores a second click while the first is still working", async () => {
    await mountIt();
    let release!: () => void;
    ipc.setupLoginItemEnable.mockImplementation(
      () => new Promise<void>((resolve) => (release = resolve)),
    );
    const first = api.enableLoginItem();
    await api.enableLoginItem();
    expect(ipc.setupLoginItemEnable).toHaveBeenCalledTimes(1);
    release();
    await first;
  });
});
