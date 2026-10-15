import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { ref } from "vue";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import CursorDesktopSettings from "./CursorDesktopSettings.vue";
import { i18n } from "../../i18n";

const api = vi.hoisted(() => ({ status: vi.fn(), persist: vi.fn(), context: vi.fn(), mac: true }));
vi.mock("../../lib/ipc", () => ({ cursorDesktopStatus: api.status }));
vi.mock("./context", () => ({ useSettingsContext: api.context }));
vi.mock("../../lib/platform", () => ({ get isMac() { return api.mac; } }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

let wrapper: VueWrapper | undefined;
const config = ref({ cursorDesktop: { suppressBackgroundCompletionFollowUp: false } });
const state = (enabled = false, phase = enabled ? "active" : "disabled") => ({
  supported: true, preferenceEnabled: enabled, enabled, active: phase === "active", phase,
  processes: 0, windows: [],
});
beforeEach(() => {
  api.mac = true;
  api.status.mockReset().mockResolvedValue(state());
  api.persist.mockReset().mockResolvedValue(undefined);
  config.value.cursorDesktop.suppressBackgroundCompletionFollowUp = false;
  api.context.mockReturnValue({ config, persist: api.persist, modes: ref({ cursor: { mode: "mcp" } }), modeBusy: ref({ cursor: false }) });
  i18n.global.locale.value = "en";
});
afterEach(() => { wrapper?.unmount(); wrapper = undefined; vi.useRealTimers(); });
async function mountRow() {
  wrapper = mount(CursorDesktopSettings, { global: { plugins: [i18n] } });
  await flushPromises();
  return wrapper;
}

it("defaults off and explains the global Desktop scope", async () => {
  const row = await mountRow();
  expect((row.get("input").element as HTMLInputElement).checked).toBe(false);
  expect(row.get('[role="status"]').text()).toBe("Off");
  expect(row.text()).toContain("all local Cursor Desktop chats");
  expect(row.text()).toContain("Terminal CLI is not covered");
  expect(row.text()).toContain("experimental");
  expect(row.text()).toContain("after checks pass");
  expect(api.persist).not.toHaveBeenCalled();
});

it("distinguishes saved preference from incomplete runtime coverage", async () => {
  api.status.mockResolvedValue({ ...state(true, "error"), error: "IDE adapter is unavailable" });
  const row = await mountRow();
  expect((row.get("input").element as HTMLInputElement).checked).toBe(true);
  expect(row.get('[role="status"]').text()).toBe("Unable to activate in all Cursor Desktop windows");
  expect(row.get('[role="alert"]').text()).toContain("IDE adapter is unavailable");
});

it("rolls back an unsuccessful preference save", async () => {
  const row = await mountRow();
  api.persist.mockRejectedValueOnce(new Error("save failed"));
  await row.get("input").setValue(true);
  await flushPromises();
  expect(config.value.cursorDesktop.suppressBackgroundCompletionFollowUp).toBe(false);
  expect(row.get('[role="alert"]').text()).toContain("save failed");
});

it("ignores an older poll after saving and immediately refreshes the new state", async () => {
  const row = await mountRow();
  let finish!: (value: ReturnType<typeof state>) => void;
  api.status.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  window.dispatchEvent(new Event("focus"));
  api.status.mockResolvedValue(state(true));
  await row.get("input").setValue(true);
  await flushPromises();
  finish(state());
  await flushPromises();
  expect(config.value.cursorDesktop.suppressBackgroundCompletionFollowUp).toBe(true);
  expect(row.get('[role="status"]').text()).toBe("Active in Cursor Desktop");
});

it("does not offer activation on unsupported platforms", async () => {
  api.mac = false;
  const row = await mountRow();
  expect(row.get("input").attributes("disabled")).toBeDefined();
  expect(row.get('[role="status"]').text()).toContain("macOS only");
});

it("clears a restoration error when an automatic retry succeeds after disabling", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  api.status.mockResolvedValue({ ...state(false, "error"), error: "Restoration is pending" });
  const row = await mountRow();
  expect(row.find('[role="alert"]').exists()).toBe(true);
  api.status.mockResolvedValue(state());
  vi.advanceTimersByTime(3000);
  await flushPromises();
  expect(row.get('[role="status"]').text()).toBe("Off");
  expect(row.find('[role="alert"]').exists()).toBe(false);
});
