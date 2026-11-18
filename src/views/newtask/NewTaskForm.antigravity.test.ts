// The New Task form with an Antigravity task: it runs in the App with the folder's allowed actions
// only, so there is no permission to choose, and its create receipt keeps a retry from starting
// the task twice (docs/specs/antigravity-deep-integration.md §12).
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import NewTaskForm from "./NewTaskForm.vue";
import { i18n } from "../../i18n";
import type { NewTaskOptions } from "../../lib/types";

const api = vi.hoisted(() => ({
  launch: vi.fn(),
  options: vi.fn(),
  route: vi.fn(),
  init: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock("../../lib/ipc", () => ({
  agentTaskReadiness: vi.fn(async () => [
    {
      kind: "antigravity",
      label: "Antigravity",
      ready: false,
      binaryReady: false,
      integrationReady: true,
      lifecycleReady: true,
      integrationMode: "mcp",
      executable: null,
      launchSupported: true,
    },
  ]),
  codexLaunchStatus: vi.fn(async () => null),
  cursorLaunchStatus: vi.fn(async () => null),
  antigravityLaunchStatus: api.route,
  newTaskOptions: api.options,
  newTaskInit: api.init,
  newTaskLaunch: api.launch,
  newTaskProjects: vi.fn(async () => []),
  newTaskProjectsRefreshed: vi.fn(async () => []),
  openPath: vi.fn(),
  openSettings: vi.fn(),
  projectKeyOf: vi.fn(async () => "/project"),
  todosList: vi.fn(async () => []),
}));

/** A flat model list with the tier folded into the id and no effort row, as `agy models` gives. */
function options(): NewTaskOptions {
  return {
    speedControl: false,
    maxControl: false,
    listed: true,
    models: [
      { id: "gemini-flash-low", label: "Gemini Flash (low)", efforts: [] },
      { id: "gemini-flash-high", label: "Gemini Flash (high)", efforts: [] },
    ],
    default: { label: "Antigravity's own setting", efforts: [] },
  };
}

function route(patch: Record<string, unknown> = {}) {
  return {
    preference: "desktop",
    target: "desktop",
    desktopAvailable: true,
    terminalAvailable: false,
    desktopReason: null,
    terminalReason: "cliNotDetected",
    integrated: true,
    ...patch,
  };
}

let wrapper: VueWrapper;
beforeEach(() => {
  localStorage.clear();
  api.launch.mockReset().mockResolvedValue(undefined);
  api.options.mockReset().mockResolvedValue(options());
  api.route.mockReset().mockResolvedValue(route());
  api.init.mockReset().mockResolvedValue({ popupSubmitKey: "cmdEnter", permissionPrompt: "ask" });
  i18n.global.locale.value = "en";
});
afterEach(() => wrapper?.unmount());

async function mountForm() {
  wrapper = mount(NewTaskForm, { props: { lockProject: "/project" }, global: { plugins: [i18n] } });
  await flushPromises();
}
async function form() {
  await mountForm();
  await wrapper.get(".nt-agent").trigger("click");
  await flushPromises();
  await wrapper.get("textarea").setValue("Task");
}
const launchButton = () => wrapper.get(".nt-btn-launch");
async function start() {
  await launchButton().trigger("click");
  await flushPromises();
}

it("lists Antigravity with the route it will use, selectable although it has no agy", async () => {
  await mountForm();
  const row = wrapper.get(".nt-agent");
  expect(row.classes()).not.toContain("disabled");
  expect(row.text()).toContain("Antigravity");
  expect(row.text()).toContain("New tasks will use the Antigravity App");
});

/** Picks one of the two permission options the form offers when the setting asks every time. */
async function choosePermission(label: "Agent default" | "YOLO") {
  const choice = wrapper.findAll(".nt-choice").find((item) => item.text().includes(label));
  expect(choice, label).toBeDefined();
  await choice!.trigger("click");
}

it("asks for the permission like the other Agents and says what Agent default means in the App", async () => {
  await form();
  expect(wrapper.text()).toContain("Agent default");
  expect(wrapper.text()).toContain("YOLO");
  expect(wrapper.get("[data-testid=nt-antigravity-permission-hint]").text()).toContain("lets the task do only what the folder allows");
  // Nothing is preselected: the task waits for the user's choice.
  expect(launchButton().attributes("disabled")).toBeDefined();
  await choosePermission("Agent default");
  expect(launchButton().attributes("disabled")).toBeUndefined();
});

it("does not explain the App's whitelist when the task will run in agy", async () => {
  api.route.mockResolvedValue(route({ preference: "terminal", target: "terminal", desktopAvailable: false, terminalAvailable: true, desktopReason: "sidecarsNotAllowed", terminalReason: null }));
  await form();
  expect(wrapper.find("[data-testid=nt-antigravity-permission-hint]").exists()).toBe(false);
  expect(wrapper.text()).toContain("YOLO");
});

it("starts the task with the Agent default permission and the model picked from the flat list", async () => {
  await form();
  expect(wrapper.find("#nt-task-effort").exists()).toBe(false);
  await wrapper.get("#nt-task-model").setValue("gemini-flash-low");
  await choosePermission("Agent default");
  await start();
  expect(api.launch).toHaveBeenCalledTimes(1);
  expect(api.launch.mock.calls[0][0]).toMatchObject({
    kind: "antigravity",
    workspace: "/project",
    task: "Task",
    permission: "agent-default",
    model: "gemini-flash-low",
  });
  expect(api.launch.mock.calls[0][0].effort).toBeNull();
  expect(wrapper.emitted("launched")).toHaveLength(1);
});

it("starts a YOLO task with the YOLO permission", async () => {
  await form();
  await choosePermission("YOLO");
  await start();
  expect(api.launch.mock.calls[0][0]).toMatchObject({ kind: "antigravity", permission: "yolo" });
});

it("says under the form that the task is created in the App", async () => {
  await form();
  expect(wrapper.get(".nt-footer").text()).toContain("creates a conversation in the Antigravity App");
});

it("shows why it cannot start, instead of the CLI's conditions, and cannot be picked", async () => {
  api.route.mockResolvedValue(
    route({ target: null, desktopAvailable: false, desktopReason: "sidecarsNotAllowed" }),
  );
  await mountForm();
  const row = wrapper.get(".nt-agent");
  expect(row.classes()).toContain("disabled");
  expect(row.text()).toContain("not allowed to manage Antigravity sidecars");
  expect(row.find(".nt-issue").exists()).toBe(false);
  await row.trigger("click");
  await flushPromises();
  expect(wrapper.find("[data-testid=nt-task-settings]").exists()).toBe(false);
});

it("keeps one operation ID for a retry, so a task that may exist is not created twice", async () => {
  api.launch.mockRejectedValue(new Error("Operation outcome is unknown"));
  await form();
  await choosePermission("Agent default");
  await start();
  const id = api.launch.mock.calls[0][0].operationId;
  expect(localStorage.getItem("antigravity-desktop-pending-create")).toContain(id);
  expect(wrapper.get("textarea").element.value).toBe("Task");
  expect(wrapper.emitted("launched")).toBeUndefined();

  await start();
  expect(api.launch.mock.calls[1][0].operationId).toBe(id);

  api.launch.mockResolvedValue(undefined);
  await start();
  expect(api.launch.mock.calls[2][0].operationId).toBe(id);
  expect(wrapper.emitted("launched")).toHaveLength(1);
  expect(localStorage.getItem("antigravity-desktop-pending-create")).toBeNull();
});

it("does not reuse the ID once the task changed", async () => {
  api.launch.mockRejectedValue(new Error("No task was submitted"));
  await form();
  await choosePermission("Agent default");
  await start();
  const first = api.launch.mock.calls[0][0].operationId;
  await wrapper.get("textarea").setValue("A different task");
  await start();
  expect(api.launch.mock.calls[1][0].operationId).not.toBe(first);
});
