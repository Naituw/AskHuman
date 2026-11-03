import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { ref } from "vue";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import CursorDesktopSettings from "./CursorDesktopSettings.vue";
import { i18n } from "../../i18n";

const api = vi.hoisted(() => ({ status: vi.fn(), launch: vi.fn(), persist: vi.fn(), context: vi.fn(), mac: true }));
vi.mock("../../lib/ipc", () => ({ cursorDesktopStatus: api.status, cursorLaunchStatus: api.launch }));
vi.mock("./context", () => ({ useSettingsContext: api.context }));
vi.mock("../../lib/platform", () => ({ get isMac() { return api.mac; } }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

let wrapper: VueWrapper | undefined;
const config = ref({ cursorDesktop: { suppressBackgroundCompletionFollowUp: false, launchPreference: "terminal" } });
const launch = (preference = "terminal", target: string | null = preference) => ({
  preference, target, desktopAvailable: target === "desktop", terminalAvailable: true, integrated: true,
  desktopReason: preference === "desktop" && target !== "desktop" ? "appNotRunning" : null, terminalReason: null,
});
const state = (enabled = false, phase = enabled ? "active" : "disabled") => ({
  supported: true, preferenceEnabled: enabled, enabled, active: phase === "active", phase,
  processes: 0, windows: [],
});
beforeEach(() => {
  api.mac = true;
  api.status.mockReset().mockResolvedValue(state());
  api.launch.mockReset().mockResolvedValue(launch());
  api.persist.mockReset().mockResolvedValue(undefined);
  config.value.cursorDesktop.suppressBackgroundCompletionFollowUp = false;
  config.value.cursorDesktop.launchPreference = "terminal";
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

// --- where new Cursor tasks run (docs/specs/cursor-desktop-task-creation.md §3) ---

const launchButtons = (row: VueWrapper) => row.findAll(".segmented button");
const launchStatusText = (row: VueWrapper) => row.findAll('[role="status"]')[1].text();

it("shows the route new tasks will use, with the terminal as the default", async () => {
  const row = await mountRow();
  expect(launchButtons(row)).toHaveLength(2);
  expect(launchButtons(row)[0].attributes("aria-pressed")).toBe("true");
  expect(launchButtons(row)[1].attributes("aria-pressed")).toBe("false");
  expect(launchStatusText(row)).toBe("New tasks will use the CLI");
  expect(api.persist).not.toHaveBeenCalled();
});

it("saves the Agents window as the preference and re-reads what that resolves to", async () => {
  const row = await mountRow();
  api.launch.mockResolvedValue(launch("desktop"));
  await launchButtons(row)[1].trigger("click");
  await flushPromises();
  expect(api.persist).toHaveBeenCalledTimes(1);
  expect(config.value.cursorDesktop.launchPreference).toBe("desktop");
  expect(launchStatusText(row)).toBe("New tasks will use Cursor's Agents window");
});

it("explains why a preferred Agents window is not what will be used", async () => {
  config.value.cursorDesktop.launchPreference = "desktop";
  api.launch.mockResolvedValue(launch("desktop", "terminal"));
  const row = await mountRow();
  // The preference is kept while the fallback and its cause are shown.
  expect(launchButtons(row)[1].attributes("aria-pressed")).toBe("true");
  expect(launchStatusText(row)).toBe("New tasks will use the CLI (Cursor is not running)");
  expect(row.find("button.refresh-runtime").exists()).toBe(true);
});

it("rolls back a preference that could not be saved", async () => {
  const row = await mountRow();
  api.persist.mockRejectedValueOnce(new Error("save failed"));
  await launchButtons(row)[1].trigger("click");
  await flushPromises();
  expect(config.value.cursorDesktop.launchPreference).toBe("terminal");
  const alerts = row.findAll('[role="alert"]');
  expect(alerts[alerts.length - 1].text()).toContain("save failed");
});

it("does not offer the Agents window where it cannot exist", async () => {
  api.mac = false;
  const row = await mountRow();
  expect(launchButtons(row)[1].attributes("disabled")).toBeDefined();
  expect(launchButtons(row)[0].attributes("disabled")).toBeUndefined();
});

it("reconciles a preference changed elsewhere when the window regains focus", async () => {
  const row = await mountRow();
  api.launch.mockResolvedValue(launch("desktop"));
  window.dispatchEvent(new Event("focus"));
  await flushPromises();
  expect(config.value.cursorDesktop.launchPreference).toBe("desktop");
  expect(launchButtons(row)[1].attributes("aria-pressed")).toBe("true");
});
