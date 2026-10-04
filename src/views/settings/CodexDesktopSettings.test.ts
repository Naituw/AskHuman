import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { ref } from "vue";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import CodexDesktopSettings from "./CodexDesktopSettings.vue";
import { i18n } from "../../i18n";
const api = vi.hoisted(() => ({ status: vi.fn(), persist: vi.fn(), context: vi.fn(), listeners: new Map<string, () => void>() }));
vi.mock("../../lib/ipc", () => ({ codexLaunchStatus: api.status }));
vi.mock("./context", () => ({ useSettingsContext: api.context }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, callback) => { api.listeners.set(name, callback); return () => api.listeners.delete(name); }) }));
let wrapper: VueWrapper;
const config = ref({codexDesktop: {appPath:"",codexHome:"",launchPreference:"desktop"}});
const status = (preference = "desktop", target: string | null = "desktop") => ({ preference, target, desktopAvailable: target === "desktop", terminalAvailable: true, integrated: true, desktopReason: target === "terminal" && preference === "desktop" ? "appNotDetected" : null, terminalReason: null });
beforeEach(() => {
  api.status.mockReset().mockResolvedValue(status());
  api.persist.mockReset().mockResolvedValue(undefined);
  config.value.codexDesktop.launchPreference = "desktop";
  api.context.mockReturnValue({config,persist:api.persist,modes:ref({codex:{mode:"mcp"}}),modeBusy:ref({codex:false})});
  i18n.global.locale.value = "en";
});
afterEach(() => wrapper?.unmount());
async function mountRow() { wrapper = mount(CodexDesktopSettings, {global:{plugins:[i18n]}}); await flushPromises(); }
it("shows one preference row and effective runtime without connection controls", async () => {
  await mountRow();
  expect(wrapper.findAll("button")).toHaveLength(2);
  expect(wrapper.findAll("input,select,details")).toHaveLength(0);
  expect(wrapper.get('[role="status"]').text()).toBe("New tasks will use Desktop App");
  expect(wrapper.findAll("button")[0].attributes("aria-pressed")).toBe("true");
});
it("reconciles external preference changes on focus and settings events", async () => {
  await mountRow();
  api.status.mockResolvedValue(status("terminal", "terminal"));
  window.dispatchEvent(new Event("focus")); await flushPromises();
  expect(config.value.codexDesktop.launchPreference).toBe("terminal");
  expect(wrapper.get('[role="status"]').text()).toContain("CLI");
  api.status.mockResolvedValue(status()); api.listeners.get("settings-updated")!(); await flushPromises();
  expect(config.value.codexDesktop.launchPreference).toBe("desktop");
});
it("ignores a stale detection response while saving a new preference", async () => {
  await mountRow();
  let finish!: (value: ReturnType<typeof status>) => void;
  api.status.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
  window.dispatchEvent(new Event("focus"));
  api.status.mockResolvedValue(status("terminal", "terminal"));
  await wrapper.findAll("button")[1].trigger("click"); await flushPromises();
  finish(status()); await flushPromises();
  expect(api.persist).toHaveBeenCalledTimes(1);
  expect(config.value.codexDesktop.launchPreference).toBe("terminal");
  expect(wrapper.get('[role="status"]').text()).toContain("CLI");
});
it("preserves preference while showing fallback and rolls back a failed save", async () => {
  api.status.mockResolvedValue(status("desktop", "terminal")); await mountRow();
  expect(wrapper.findAll("button")[0].attributes("aria-pressed")).toBe("true");
  expect(wrapper.get('[role="status"]').text()).toContain("CLI");
  api.persist.mockRejectedValueOnce(new Error("save failed"));
  await wrapper.findAll("button")[1].trigger("click"); await flushPromises();
  expect(config.value.codexDesktop.launchPreference).toBe("desktop");
  expect(wrapper.get('[role="alert"]').text()).toContain("save failed");
});

it("shows a fallback cause and refreshes without changing the preference", async () => {
  api.status.mockResolvedValue(status("desktop", "terminal")); await mountRow();
  expect(wrapper.get('[role="status"]').text()).toBe("New tasks will use CLI (Desktop App was not detected)");
  const refresh = () => wrapper.findAll("button").find(b => b.classes().includes("refresh-runtime"))!;
  let finish!: (value: ReturnType<typeof status>) => void;
  api.status.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await refresh().trigger("click");
  expect(refresh().attributes("disabled")).toBeDefined();
  expect(refresh().text()).toBe("Checking…");
  expect(api.status).toHaveBeenLastCalledWith(true);
  finish(status("desktop", "terminal")); await flushPromises();
  expect(refresh().attributes("disabled")).toBeUndefined();
  expect(wrapper.get('[role="status"]').text()).toContain("Desktop App was not detected");
  api.status.mockResolvedValue(status()); await refresh().trigger("click"); await flushPromises();
  expect(wrapper.find(".refresh-runtime").exists()).toBe(false);
  expect(wrapper.get('[role="status"]').text()).toBe("New tasks will use Desktop App");
  expect(config.value.codexDesktop.launchPreference).toBe("desktop");
  expect(api.persist).not.toHaveBeenCalled();
});

it("shows detection feedback on both preference transitions instead of the previous result", async () => {
  api.status.mockResolvedValue(status("desktop", "terminal")); await mountRow();
  for (const preference of ["terminal", "desktop"] as const) {
    let finish!: (value: ReturnType<typeof status>) => void;
    api.status.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
    await wrapper.findAll("button")[preference === "terminal" ? 1 : 0].trigger("click");
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe("Checking…");
    expect(wrapper.find(".refresh-runtime").exists()).toBe(false);
    finish(status(preference, "terminal")); await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe(preference === "terminal" ? "New tasks will use CLI" : "New tasks will use CLI (Desktop App was not detected)");
  }
});
it("ends detection feedback on failure instead of leaving stale results or an infinite spinner", async () => {
  await mountRow();
  api.status.mockRejectedValueOnce(new Error("unavailable"));
  await wrapper.findAll("button")[1].trigger("click"); await flushPromises();
  expect(wrapper.get('[role="status"]').text()).toBe("Unable to complete detection");
  expect(wrapper.get('[role="alert"]').text()).toContain("unavailable");
});
