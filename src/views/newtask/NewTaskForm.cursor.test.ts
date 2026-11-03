import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import NewTaskForm from "./NewTaskForm.vue";
import { i18n } from "../../i18n";
import type { CursorTaskOptions } from "../../lib/types";

const api = vi.hoisted(() => ({
  launch: vi.fn(),
  options: vi.fn(),
  readiness: vi.fn(),
  route: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock("../../lib/ipc", () => ({
  agentTaskReadiness: api.readiness,
  codexLaunchStatus: vi.fn(async () => null),
  cursorLaunchStatus: api.route,
  newTaskCursorOptions: api.options,
  newTaskInit: vi.fn(async () => ({
    popupSubmitKey: "cmdEnter",
    permissionPrompt: "agent-default",
  })),
  newTaskLaunch: api.launch,
  newTaskProjects: vi.fn(async () => []),
  newTaskProjectsRefreshed: vi.fn(async () => []),
  openPath: vi.fn(),
  openSettings: vi.fn(),
  projectKeyOf: vi.fn(async () => "/project"),
  todosList: vi.fn(async () => []),
}));

/** Synthetic model list: nothing here comes from a real Cursor installation. */
function options(patch: Partial<CursorTaskOptions> = {}): CursorTaskOptions {
  return {
    planKnown: true,
    listed: true,
    models: [
      { id: "small", label: "Small", maxRequired: false },
      { id: "big-max", label: "Big Max", maxRequired: true },
      { id: "fresh", label: "Fresh" },
    ],
    default: { label: "Big Max", maxRequired: true },
    ...patch,
  };
}

/** Cursor as the readiness probe reports it; the CLI can be made unready. */
function cursorReadiness(ready = true) {
  return [
    {
      kind: "cursor",
      label: "Cursor",
      ready,
      binaryReady: ready,
      integrationReady: true,
      lifecycleReady: ready,
      integrationMode: "hook",
      executable: "/bin/cursor-agent",
      launchSupported: true,
    },
  ];
}

/** Where a Cursor task would run, as the backend reports it. */
function route(preference: "terminal" | "desktop", target: "terminal" | "desktop" | null = preference) {
  return {
    preference,
    target,
    desktopAvailable: target === "desktop",
    terminalAvailable: target === "terminal",
    desktopReason: null,
    terminalReason: null,
    integrated: true,
  };
}

let wrapper: VueWrapper;
beforeEach(() => {
  localStorage.clear();
  api.launch.mockReset().mockResolvedValue(undefined);
  api.options.mockReset().mockResolvedValue(options());
  api.readiness.mockReset().mockResolvedValue(cursorReadiness());
  // Unknown route: Cursor is judged by its own readiness, as it was before the Agents window.
  api.route.mockReset().mockResolvedValue(null);
  i18n.global.locale.value = "en";
});
afterEach(() => wrapper?.unmount());

async function form() {
  wrapper = mount(NewTaskForm, {
    props: { lockProject: "/project" },
    global: { plugins: [i18n] },
  });
  await flushPromises();
  await wrapper.get(".nt-agent").trigger("click");
  await flushPromises();
  await wrapper.get("textarea").setValue("Task");
}

const section = () => wrapper.find("[data-testid=nt-cursor-max]");
const launchButton = () => wrapper.get(".nt-btn-launch");
const allowSwitch = () => wrapper.get(".nt-allow-max");

it("shows nothing extra while Max is not controlled and passes no Max fields", async () => {
  api.options.mockResolvedValue(null);
  await form();
  expect(section().exists()).toBe(false);
  expect(launchButton().attributes("disabled")).toBeUndefined();
  await launchButton().trigger("click");
  await flushPromises();
  expect(api.launch).toHaveBeenCalledTimes(1);
  const payload = api.launch.mock.calls[0][0];
  expect(payload).toMatchObject({ kind: "cursor", workspace: "/project", task: "Task" });
  expect(payload.model).toBeNull();
  expect(payload.allowMax).toBeUndefined();
});

it("blocks a default that needs Max until the task allows it", async () => {
  await form();
  expect(section().exists()).toBe(true);
  expect(wrapper.get(".nt-will-use").text()).toBe("Will use: Big Max · Needs Max");
  expect(wrapper.get("[data-testid=nt-max-blocked]").text()).toContain("needs Max");
  expect(launchButton().attributes("disabled")).toBeDefined();
  // Off by default and never pre-armed.
  expect(allowSwitch().attributes("aria-checked")).toBe("false");

  await allowSwitch().trigger("click");
  expect(allowSwitch().attributes("aria-checked")).toBe("true");
  expect(wrapper.find("[data-testid=nt-max-blocked]").exists()).toBe(false);
  expect(launchButton().attributes("disabled")).toBeUndefined();
  await launchButton().trigger("click");
  await flushPromises();
  expect(api.launch.mock.calls[0][0]).toMatchObject({ allowMax: true, model: null });
});

it("launches a model that does not need Max with Max left off", async () => {
  await form();
  await wrapper.get("#nt-cursor-model").setValue("small");
  expect(wrapper.get(".nt-will-use").text()).toBe("Will use: Small · No Max needed");
  expect(wrapper.find("[data-testid=nt-max-blocked]").exists()).toBe(false);
  await launchButton().trigger("click");
  await flushPromises();
  expect(api.launch.mock.calls[0][0]).toMatchObject({ model: "small", allowMax: false });
});

it("marks every listed model and leaves the unknown one unmarked", async () => {
  await form();
  const labels = wrapper.findAll("#nt-cursor-model option").map((o) => o.text());
  expect(labels).toEqual([
    "Follow Cursor's default",
    "Small · No Max needed",
    "Big Max · Needs Max",
    "Fresh",
  ]);
});

it("lets an unknown verdict through with a note", async () => {
  await form();
  await wrapper.get("#nt-cursor-model").setValue("fresh");
  expect(wrapper.get(".nt-will-use").text()).toBe("Will use: Fresh · unknown");
  expect(wrapper.get("[data-testid=nt-max-unknown]").text()).toContain("could not be determined");
  expect(launchButton().attributes("disabled")).toBeUndefined();
});

it("falls back to Cursor's default when its model list cannot be read", async () => {
  api.options.mockResolvedValue(
    options({ listed: false, models: [], default: { maxRequired: false } }),
  );
  await form();
  expect(wrapper.get("#nt-cursor-model").attributes("disabled")).toBeDefined();
  expect(wrapper.text()).toContain("model list could not be read");
  // The default has no known name, so the line says what it stands for.
  expect(wrapper.get(".nt-will-use").text()).toBe(
    "Will use: Cursor's current default model · No Max needed",
  );
  expect(launchButton().attributes("disabled")).toBeUndefined();
});

it("does not carry a model or the Max switch over to the next task", async () => {
  await form();
  await wrapper.get("#nt-cursor-model").setValue("small");
  await allowSwitch().trigger("click");
  await launchButton().trigger("click");
  await flushPromises();
  expect(api.launch).toHaveBeenCalledTimes(1);
  expect((wrapper.get("#nt-cursor-model").element as HTMLSelectElement).value).toBe("");
  expect(allowSwitch().attributes("aria-checked")).toBe("false");
});

it("drops a model that the refreshed list no longer offers", async () => {
  await form();
  await wrapper.get("#nt-cursor-model").setValue("small");
  api.options.mockResolvedValue(options({ models: [{ id: "other", label: "Other", maxRequired: false }] }));
  window.dispatchEvent(new Event("focus"));
  await flushPromises();
  expect((wrapper.get("#nt-cursor-model").element as HTMLSelectElement).value).toBe("");
});

it("hides the section again when Max stops being controlled", async () => {
  await form();
  expect(section().exists()).toBe(true);
  api.options.mockResolvedValue(null);
  window.dispatchEvent(new Event("focus"));
  await flushPromises();
  expect(section().exists()).toBe(false);
  expect(launchButton().attributes("disabled")).toBeUndefined();
});

// --- where the task runs (docs/specs/cursor-desktop-task-creation.md §3) ---

/** A default that needs no Max, so Max control does not block the launch under test. */
const freeDefault = () => options({ default: { label: "Small", maxRequired: false } });

it("keeps the terminal route silent: nothing about the Agents window is said", async () => {
  api.route.mockResolvedValue(route("terminal"));
  await form();
  expect(wrapper.get(".nt-agent").text()).not.toContain("Agents window");
  expect(wrapper.get(".nt-agent").text()).toContain("/bin/cursor-agent");
  expect(wrapper.get(".nt-footer .nt-note").text()).toBe("Submitting starts the Agent in a new Terminal window.");
});

it("says the task goes to the Agents window when that is the route", async () => {
  api.route.mockResolvedValue(route("desktop"));
  await form();
  expect(wrapper.get(".nt-agent").text()).toContain("New tasks will use Cursor's Agents window");
  expect(wrapper.get(".nt-footer .nt-note").text()).toContain("Agents window");
});

it("can start through the Agents window although the CLI is not ready", async () => {
  api.options.mockResolvedValue(freeDefault());
  api.readiness.mockResolvedValue(cursorReadiness(false));
  api.route.mockResolvedValue(route("desktop"));
  await form();
  expect(wrapper.get(".nt-agent").classes()).not.toContain("disabled");
  await launchButton().trigger("click");
  await flushPromises();
  expect(api.launch).toHaveBeenCalledTimes(1);
  expect(api.launch.mock.calls[0][0]).toMatchObject({ kind: "cursor", workspace: "/project" });
});

it("treats a Cursor with no route at all as not startable", async () => {
  api.readiness.mockResolvedValue(cursorReadiness(false));
  api.route.mockResolvedValue({ ...route("desktop", null), desktopReason: "appNotRunning", terminalReason: "cliNotDetected" });
  wrapper = mount(NewTaskForm, { props: { lockProject: "/project" }, global: { plugins: [i18n] } });
  await flushPromises();
  expect(wrapper.get(".nt-agent").classes()).toContain("disabled");
  expect(wrapper.get(".nt-agent").text()).toContain("Unable to start: Cursor is not running; Cursor CLI was not detected");
});

it("does not blame the Agents window when the terminal was the choice", async () => {
  api.readiness.mockResolvedValue(cursorReadiness(false));
  api.route.mockResolvedValue({ ...route("terminal", null), desktopReason: "appNotRunning", terminalReason: "cliNotDetected" });
  wrapper = mount(NewTaskForm, { props: { lockProject: "/project" }, global: { plugins: [i18n] } });
  await flushPromises();
  expect(wrapper.get(".nt-agent").classes()).toContain("disabled");
  expect(wrapper.get(".nt-agent").text()).toContain("Unable to start: Cursor CLI was not detected");
  expect(wrapper.get(".nt-agent").text()).not.toContain("Cursor is not running");
});

it("retries a failed launch under the same operation id, and a changed model is a new operation", async () => {
  api.options.mockResolvedValue(freeDefault());
  api.route.mockResolvedValue(route("desktop"));
  api.launch.mockRejectedValue(new Error("unknown outcome"));
  await form();
  await launchButton().trigger("click");
  await flushPromises();
  const first = api.launch.mock.calls[0][0].operationId;
  expect(localStorage.getItem("cursor-desktop-pending-create")).toContain(first);
  await launchButton().trigger("click");
  await flushPromises();
  expect(api.launch.mock.calls[1][0].operationId).toBe(first);
  await wrapper.get("#nt-cursor-model").setValue("small");
  await launchButton().trigger("click");
  await flushPromises();
  expect(api.launch.mock.calls[2][0].operationId).not.toBe(first);
  api.launch.mockResolvedValue(undefined);
  await launchButton().trigger("click");
  await flushPromises();
  expect(wrapper.emitted("launched")).toHaveLength(1);
  expect(localStorage.getItem("cursor-desktop-pending-create")).toBeNull();
});
