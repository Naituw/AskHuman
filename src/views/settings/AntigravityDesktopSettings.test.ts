// The Antigravity card's task-creation part: the allowance for AskHuman to manage sidecars, which
// route a new task tries first, and the sidecars it keeps (docs/specs/antigravity-deep-integration.md
// §12.10).
import { flushPromises, mount, type DOMWrapper, type VueWrapper } from "@vue/test-utils";
import { ref } from "vue";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import AntigravityDesktopSettings from "./AntigravityDesktopSettings.vue";
import { i18n } from "../../i18n";
import type { AntigravitySidecar } from "../../lib/ipc";

const api = vi.hoisted(() => ({
  launch: vi.fn(),
  desktop: vi.fn(),
  persist: vi.fn(),
  context: vi.fn(),
  mac: true,
}));
vi.mock("../../lib/ipc", () => ({ antigravityLaunchStatus: api.launch, antigravityDesktop: api.desktop }));
vi.mock("./context", () => ({ useSettingsContext: api.context }));
vi.mock("../../lib/platform", () => ({
  get isMac() {
    return api.mac;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

let wrapper: VueWrapper | undefined;
const config = ref<{ antigravityDesktop: { launchPreference: string; manageSidecars: boolean } }>({
  antigravityDesktop: { launchPreference: "desktop", manageSidecars: false },
});
/** What the daemon would list; tests change it before they act. */
let sidecars: AntigravitySidecar[] = [];

const sidecar = (patch: Partial<AntigravitySidecar> = {}): AntigravitySidecar => ({
  id: "askhuman-ws-0a1b2c3d",
  kind: "workspace",
  directory: "/Users/me/project",
  directoryExists: true,
  rules: [],
  createdAt: 1_700_000_000_000,
  lastUsedAt: 1_700_000_500_000,
  files: "present",
  enabled: true,
  tracked: true,
  online: true,
  ...patch,
});
const launch = (preference = "desktop", target: string | null = preference, patch: Record<string, unknown> = {}) => ({
  preference,
  target,
  desktopAvailable: target === "desktop",
  terminalAvailable: target === "terminal",
  desktopReason: null,
  terminalReason: "cliNotDetected",
  integrated: true,
  ...patch,
});

beforeEach(() => {
  api.mac = true;
  sidecars = [];
  config.value.antigravityDesktop = { launchPreference: "desktop", manageSidecars: false };
  api.launch.mockReset().mockResolvedValue(launch());
  api.persist.mockReset().mockResolvedValue(undefined);
  api.desktop.mockReset().mockImplementation(async (operation: { op: string }) =>
    operation.op === "sidecars"
      ? { status: "done", managing: config.value.antigravityDesktop.manageSidecars, limit: 20, sidecars }
      : { status: "done" },
  );
  api.context.mockReturnValue({
    config,
    persist: api.persist,
    modeBusy: ref({ antigravity: false }),
  });
  i18n.global.locale.value = "en";
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
});

async function mountRow() {
  wrapper = mount(AntigravityDesktopSettings, { global: { plugins: [i18n] } });
  await flushPromises();
  return wrapper;
}
const operations = () => api.desktop.mock.calls.map(([operation]) => operation);
const manageSwitch = (row: VueWrapper) => row.get('[data-testid="antigravity-manage"]');
const item = (row: VueWrapper, id = "askhuman-ws-0a1b2c3d") => row.find(`[data-testid="antigravity-sidecar-${id}"]`);
const buttonNamed = (row: Pick<DOMWrapper<Element>, "findAll">, text: string) => {
  const found = row.findAll("button").find((button) => button.text() === text);
  if (!found) throw new Error(`no button "${text}"`);
  return found;
};

// --- the allowance ---

it("starts with managing off, says what it allows, and has no sidecar to show", async () => {
  const row = await mountRow();
  expect((manageSwitch(row).element as HTMLInputElement).checked).toBe(false);
  expect(row.text()).toContain("Allow AskHuman to manage Antigravity sidecars");
  expect(row.text()).toContain("your other Antigravity settings and permissions are never touched");
  expect(row.find('[data-testid="antigravity-no-sidecars"]').exists()).toBe(true);
  expect(api.persist).not.toHaveBeenCalled();
  expect(operations()).toEqual([{ op: "sidecars" }]);
});

it("saves the allowance and then reads again what a new task would use and which sidecars exist", async () => {
  const row = await mountRow();
  api.launch.mockClear();
  api.desktop.mockClear();
  await manageSwitch(row).setValue(true);
  await flushPromises();
  expect(config.value.antigravityDesktop.manageSidecars).toBe(true);
  expect(api.persist).toHaveBeenCalledTimes(1);
  expect(api.launch).toHaveBeenCalledTimes(1);
  expect(operations()).toEqual([{ op: "sidecars" }]);
});

it("puts the allowance back when it could not be saved", async () => {
  const row = await mountRow();
  api.persist.mockRejectedValueOnce(new Error("save failed"));
  await manageSwitch(row).setValue(true);
  await flushPromises();
  expect(config.value.antigravityDesktop.manageSidecars).toBe(false);
  expect(row.get('[role="alert"]').text()).toContain("save failed");
});

// --- which route a task tries first ---

const buttons = (row: VueWrapper) => row.findAll(".segmented button");
const statusText = (row: VueWrapper) => row.get('[data-testid="antigravity-launch-status"]').text();

it("shows the App as the default route and what a new task will use", async () => {
  const row = await mountRow();
  expect(buttons(row)).toHaveLength(2);
  expect(buttons(row)[0].text()).toBe("Prefer the App");
  expect(buttons(row)[0].attributes("aria-pressed")).toBe("true");
  expect(buttons(row)[1].attributes("aria-pressed")).toBe("false");
  expect(statusText(row)).toBe("New tasks will use the Antigravity App");
});

it("explains why the App is not what will be used, in the words of the backend's reason", async () => {
  api.launch.mockResolvedValue(
    launch("desktop", null, { desktopReason: "sidecarsNotAllowed", terminalReason: "cliNotDetected" }),
  );
  const row = await mountRow();
  expect(statusText(row)).toBe(
    "Unable to start: AskHuman is not allowed to manage Antigravity sidecars yet (turn it on in Settings); The agy command was not detected",
  );
  expect(row.find("button.refresh-runtime").exists()).toBe(true);
});

it("saves a changed preference and re-reads what it resolves to", async () => {
  const row = await mountRow();
  api.launch.mockResolvedValue(launch("terminal", "desktop", { terminalReason: "cliNotDetected" }));
  await buttons(row)[1].trigger("click");
  await flushPromises();
  expect(config.value.antigravityDesktop.launchPreference).toBe("terminal");
  expect(api.persist).toHaveBeenCalledTimes(1);
  expect(statusText(row)).toBe("New tasks will use the Antigravity App (The agy command was not detected)");
});

it("rolls back a preference that could not be saved", async () => {
  const row = await mountRow();
  api.persist.mockRejectedValueOnce(new Error("save failed"));
  await buttons(row)[1].trigger("click");
  await flushPromises();
  expect(config.value.antigravityDesktop.launchPreference).toBe("desktop");
  const alerts = row.findAll('[role="alert"]');
  expect(alerts[alerts.length - 1].text()).toContain("save failed");
});

// --- the sidecars ---

it("lists each sidecar with its folder, kind, connection, last use and what is wrong with it", async () => {
  sidecars = [
    sidecar(),
    sidecar({
      id: "askhuman-ws-ffffffff",
      directory: "/Users/me/gone",
      directoryExists: false,
      online: false,
      lastUsedAt: 0,
      enabled: false,
    }),
  ];
  const row = await mountRow();
  expect(row.text()).toContain("Sidecars (2 of 20)");
  const first = item(row);
  expect(first.text()).toContain("project");
  expect(first.text()).toContain("/Users/me/project");
  expect(first.text()).toContain("Allowed actions only");
  expect(first.text()).toContain("Connected");
  expect(first.text()).toContain("Last used");
  const gone = item(row, "askhuman-ws-ffffffff");
  expect(gone.text()).toContain("Not connected");
  expect(gone.text()).toContain("Not used yet");
  expect(gone.text()).toContain("Folder not found");
  expect(gone.text()).toContain("Switched off in Antigravity");
});

it("marks a wide-grant sidecar and says what it allows", async () => {
  sidecars = [sidecar({ id: "askhuman-yolo-12345678", kind: "yolo" })];
  const row = await mountRow();
  expect(item(row, "askhuman-yolo-12345678").text()).toContain("Wide grant");
  expect(row.get('[data-testid="antigravity-wide-note"]').text()).toContain("without asking");
  // A whitelist has no such note.
  sidecars = [sidecar()];
  window.dispatchEvent(new Event("focus"));
  await flushPromises();
  expect(row.find('[data-testid="antigravity-wide-note"]').exists()).toBe(false);
});

it("removes one sidecar and reads the list again", async () => {
  sidecars = [sidecar()];
  const row = await mountRow();
  api.desktop.mockClear();
  sidecars = [];
  await buttonNamed(item(row), "Remove").trigger("click");
  await flushPromises();
  expect(operations()).toEqual([{ op: "remove", id: "askhuman-ws-0a1b2c3d" }, { op: "sidecars" }]);
  expect(item(row).exists()).toBe(false);
});

it("reports a removal the daemon refused instead of pretending it worked", async () => {
  sidecars = [sidecar()];
  const row = await mountRow();
  api.desktop.mockImplementation(async (operation: { op: string }) =>
    operation.op === "remove"
      ? { status: "refused", reason: "conflict", message: "Something in the folder is not AskHuman's" }
      : { status: "done", managing: false, limit: 20, sidecars },
  );
  await buttonNamed(item(row), "Remove").trigger("click");
  await flushPromises();
  expect(row.get('[role="alert"]').text()).toContain("not AskHuman's");
  expect(item(row).exists()).toBe(true);
});

it("asks before removing all of them", async () => {
  sidecars = [sidecar(), sidecar({ id: "askhuman-ws-ffffffff", directory: "/Users/me/other" })];
  const row = await mountRow();
  api.desktop.mockClear();
  await row.get('[data-testid="antigravity-remove-all"]').trigger("click");
  expect(operations()).toEqual([]);
  expect(row.get('[data-testid="antigravity-remove-all-confirm"]').text()).toBe("Remove all 2");
  // Cancelling leaves everything as it was.
  await buttonNamed(row, "Cancel").trigger("click");
  expect(row.find('[data-testid="antigravity-remove-all-confirm"]').exists()).toBe(false);
  expect(operations()).toEqual([]);

  await row.get('[data-testid="antigravity-remove-all"]').trigger("click");
  sidecars = [];
  await row.get('[data-testid="antigravity-remove-all-confirm"]').trigger("click");
  await flushPromises();
  expect(operations()).toEqual([{ op: "removeAll" }, { op: "sidecars" }]);
  expect(row.find('[data-testid="antigravity-remove-all"]').exists()).toBe(false);
});

it("names the sidecars that could not be removed", async () => {
  sidecars = [sidecar()];
  const row = await mountRow();
  api.desktop.mockImplementation(async (operation: { op: string }) =>
    operation.op === "removeAll"
      ? { status: "done", removed: [], failed: [{ id: "askhuman-ws-0a1b2c3d", message: "permission denied" }] }
      : { status: "done", managing: false, limit: 20, sidecars },
  );
  await row.get('[data-testid="antigravity-remove-all"]').trigger("click");
  await row.get('[data-testid="antigravity-remove-all-confirm"]').trigger("click");
  await flushPromises();
  expect(row.get('[role="alert"]').text()).toContain("askhuman-ws-0a1b2c3d: permission denied");
});

// --- rules ---

it("cannot edit rules until managing is allowed, and says so", async () => {
  sidecars = [sidecar()];
  const row = await mountRow();
  const rules = buttonNamed(item(row), "Rules (0)");
  expect(rules.attributes("disabled")).toBeDefined();
  expect(rules.attributes("title")).toContain("Allow AskHuman to manage Antigravity sidecars");
});

it("edits a folder's rules one per line and saves them trimmed", async () => {
  config.value.antigravityDesktop.manageSidecars = true;
  sidecars = [sidecar({ rules: ["read_file(~/notes/**)"] })];
  const row = await mountRow();
  await buttonNamed(item(row), "Rules (1)").trigger("click");
  const input = item(row).get("textarea");
  expect((input.element as HTMLTextAreaElement).value).toBe("read_file(~/notes/**)");
  expect(item(row).text()).toContain("is not accepted here");
  await input.setValue("  read_file(~/notes/**)  \n\ncommand(ls)\n");
  api.desktop.mockClear();
  await buttonNamed(item(row), "Save rules").trigger("click");
  await flushPromises();
  expect(operations()).toEqual([
    { op: "setRules", id: "askhuman-ws-0a1b2c3d", rules: ["read_file(~/notes/**)", "command(ls)"] },
    { op: "sidecars" },
  ]);
  expect(item(row).find("textarea").exists()).toBe(false);
});

it("keeps the editor open and shows the reason when a rule is not accepted", async () => {
  config.value.antigravityDesktop.manageSidecars = true;
  sidecars = [sidecar()];
  const row = await mountRow();
  await buttonNamed(item(row), "Rules (0)").trigger("click");
  await item(row).get("textarea").setValue("(*)");
  api.desktop.mockImplementation(async (operation: { op: string }) =>
    operation.op === "setRules"
      ? { status: "refused", reason: "invalidRules", message: "A rule that allows everything is not accepted" }
      : { status: "done", managing: true, limit: 20, sidecars },
  );
  await buttonNamed(item(row), "Save rules").trigger("click");
  await flushPromises();
  expect(item(row).get('[role="alert"]').text()).toContain("allows everything");
  expect((item(row).get("textarea").element as HTMLTextAreaElement).value).toBe("(*)");
});

// --- platform and language ---

it("does not talk to the daemon about sidecars off macOS, and offers neither switch nor App route", async () => {
  api.mac = false;
  const row = await mountRow();
  expect(operations()).toEqual([]);
  expect(manageSwitch(row).attributes("disabled")).toBeDefined();
  expect(row.text()).toContain("available on macOS only");
  expect(buttons(row)[0].attributes("disabled")).toBeDefined();
  expect(buttons(row)[1].attributes("disabled")).toBeUndefined();
  expect(row.text()).not.toContain("Sidecars (");
});

it("speaks Chinese for the allowance, the route and a sidecar row", async () => {
  sidecars = [sidecar()];
  const row = await mountRow();
  i18n.global.locale.value = "zh";
  await flushPromises();
  expect(row.text()).toContain("允许 AskHuman 管理 Antigravity sidecar");
  expect(statusText(row)).toBe("新任务将使用 Antigravity App");
  expect(item(row).text()).toContain("仅允许的操作");
  expect(item(row).text()).toContain("已连接");
});
