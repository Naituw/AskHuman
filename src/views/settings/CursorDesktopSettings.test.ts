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

// --- two guards under one switch (docs/specs/cursor-desktop-background-followup.md §7) ---

const retention = (state: string, error?: string) => ({
  state, error, checks: state === "active" ? 10 : 0, skipped: 0, skippedBubbles: 0, passed: 0,
  lastSkippedAt: 0, recent: [], atRisk: null, atRiskAt: 0,
});
const guarded = (completion: "active" | "error", guard: ReturnType<typeof retention>, id = 1) => ({
  pid: 7, id, phase: completion, surface: "agents", suppressed: 0,
  verified: completion === "active", checks: completion === "active" ? 6 : 0,
  error: completion === "error" ? "Unsupported background completion contract" : undefined, retention: guard,
});
const partial = (...windows: ReturnType<typeof guarded>[]) => ({ ...state(true, "partial"), processes: 1, windows });
const unloadError = "Unsupported conversation unload contract: method is missing";

it("names the running-chat protection when only that guard is missing, and still shows why", async () => {
  api.status.mockResolvedValue(partial(guarded("active", retention("unavailable", unloadError))));
  const row = await mountRow();
  expect(row.get('[role="status"]').text()).toBe("Partly active: running-chat protection is not active");
  expect(row.get('[role="alert"]').text()).toContain(unloadError);
});

it("names the background-completion guard when only that guard is missing", async () => {
  api.status.mockResolvedValue(partial(guarded("error", retention("active"))));
  const row = await mountRow();
  expect(row.get('[role="status"]').text()).toBe("Partly active: background-completion guard is not active");
  expect(row.get('[role="alert"]').text()).toContain("Unsupported background completion contract");
});

it("names both guards when different windows each lack one", async () => {
  api.status.mockResolvedValue(partial(
    guarded("active", retention("unavailable", unloadError), 1), guarded("error", retention("active"), 2)));
  const row = await mountRow();
  expect(row.get('[role="status"]').text())
    .toBe("Partly active: background-completion guard and running-chat protection are not active");
});

it("words the partly active state in Chinese", async () => {
  i18n.global.locale.value = "zh";
  api.status.mockResolvedValue(partial(guarded("active", retention("unavailable", unloadError))));
  const row = await mountRow();
  expect(row.get('[role="status"]').text()).toBe("部分生效：运行中会话保护未生效");
  api.status.mockResolvedValue(partial(
    guarded("active", retention("unavailable", unloadError), 1), guarded("error", retention("active"), 2)));
  window.dispatchEvent(new Event("focus"));
  await flushPromises();
  expect(row.get('[role="status"]').text()).toBe("部分生效：后台完成续跑拦截、运行中会话保护未生效");
});

it("does not claim a missing guard it cannot name", async () => {
  api.status.mockResolvedValue(partial(guarded("active", retention("active"))));
  const row = await mountRow();
  expect(row.get('[role="status"]').text()).toBe("Checking and connecting to Cursor Desktop…");
  expect(row.find('[role="alert"]').exists()).toBe(false);
});

it("stays fully active, and without a warning, when both guards work or one does not apply", async () => {
  api.status.mockResolvedValue({ ...state(true, "active"), processes: 1, windows: [
    guarded("active", retention("active"), 1),
    { ...guarded("active", retention("notApplicable"), 2), surface: "ide" },
  ] });
  const row = await mountRow();
  expect(row.get('[role="status"]').text()).toBe("Active in Cursor Desktop");
  expect(row.find('[role="alert"]').exists()).toBe(false);
});

it("keeps the single switch and renames it for both protections", async () => {
  const row = await mountRow();
  expect(row.findAll('input[type="checkbox"]')).toHaveLength(1);
  expect(row.text()).toContain("Cursor session protection (experimental)");
  expect(row.text()).toContain("running chat keeps its messages in memory");
  i18n.global.locale.value = "zh";
  await flushPromises();
  expect(row.text()).toContain("Cursor 会话防护（实验）");
  expect(row.text()).toContain("运行中的会话不再被释放内存中的消息");
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
