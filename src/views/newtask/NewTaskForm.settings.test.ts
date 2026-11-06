import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import NewTaskForm from "./NewTaskForm.vue";
import { i18n } from "../../i18n";
import type { NewTaskOptions } from "../../lib/types";

const api = vi.hoisted(() => ({ launch: vi.fn(), options: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock("../../lib/ipc", () => ({
  agentTaskReadiness: vi.fn(async () => ["codex", "claude", "pi"].map(kind => ({ kind, label: kind, ready: true, binaryReady: true, integrationReady: true, lifecycleReady: true, integrationMode: "mcp", launchSupported: true }))),
  codexLaunchStatus: vi.fn(async () => ({ preference: "desktop", target: "desktop", desktopAvailable: true, terminalAvailable: false, integrated: true })),
  cursorLaunchStatus: vi.fn(async () => null),
  newTaskInit: vi.fn(async () => ({ popupSubmitKey: "cmdEnter", permissionPrompt: "agent-default" })),
  newTaskOptions: api.options, newTaskLaunch: api.launch,
  newTaskProjects: vi.fn(async () => []), newTaskProjectsRefreshed: vi.fn(async () => []),
  openPath: vi.fn(), openSettings: vi.fn(), projectKeyOf: vi.fn(async () => "/project"), todosList: vi.fn(async () => []),
}));
function options(): NewTaskOptions {
  return { listed: true, speedControl: true, maxControl: false,
    models: [
      { id: "fast", label: "Fast model", efforts: ["medium", "high"], fastSupported: true },
      { id: "slow", label: "Slow model", efforts: ["low"], fastSupported: false },
    ], default: { label: "Fast model", efforts: ["medium", "high"], defaultEffort: "medium" },
  };
}
let wrapper: VueWrapper;
beforeEach(() => {
  localStorage.clear(); api.launch.mockReset().mockResolvedValue(undefined);
  api.options.mockReset().mockResolvedValue(options()); i18n.global.locale.value = "en";
});
afterEach(() => wrapper?.unmount());
async function form() {
  wrapper = mount(NewTaskForm, { props: { lockProject: "/project" }, global: { plugins: [i18n] } });
  await flushPromises(); await wrapper.findAll(".nt-agent")[0].trigger("click"); await flushPromises();
  await wrapper.get("textarea").setValue("Task");
}
async function start() { await wrapper.get(".nt-btn-launch").trigger("click"); await flushPromises(); }

it("submits model, effort and Fast together and remembers only model and effort after a failure", async () => {
  api.launch.mockRejectedValue(new Error("speedUnavailable")); await form();
  await wrapper.get("#nt-task-model").setValue("fast"); await wrapper.get("#nt-task-effort").setValue("high");
  await wrapper.get(".nt-fast").trigger("click"); await start();
  expect(api.launch.mock.calls[0][0]).toMatchObject({ model: "fast", effort: "high", speed: "fast" });
  expect(wrapper.get(".nt-error").text()).toContain("selected speed is unavailable");
  wrapper.unmount(); await form();
  expect((wrapper.get("#nt-task-model").element as HTMLSelectElement).value).toBe("fast");
  expect((wrapper.get("#nt-task-effort").element as HTMLSelectElement).value).toBe("high");
  expect(wrapper.get(".nt-fast").attributes("aria-checked")).toBe("false");
});

it("sends explicit standard speed by default and keeps retry identity until a choice changes", async () => {
  api.launch.mockRejectedValue(new Error("unknown")); await form(); await start();
  const first = api.launch.mock.calls[0][0]; expect(first.speed).toBe("standard");
  await start(); expect(api.launch.mock.calls[1][0].operationId).toBe(first.operationId);
  await wrapper.get("#nt-task-model").setValue("fast"); await start();
  const modelChange = api.launch.mock.calls[2][0].operationId; expect(modelChange).not.toBe(first.operationId);
  await wrapper.get(".nt-fast").trigger("click"); await start();
  expect(api.launch.mock.calls[3][0].operationId).not.toBe(modelChange);
});

it("resets Fast when choosing an unsupported model and shows the reset reason", async () => {
  await form(); await wrapper.get("#nt-task-model").setValue("fast");
  await wrapper.get(".nt-fast").trigger("click"); await wrapper.get("#nt-task-model").setValue("slow");
  expect(wrapper.get(".nt-fast").attributes("disabled")).toBeDefined();
  expect(wrapper.get("[data-testid=nt-fast-reset]").text()).toContain("turned off");
  await start(); expect(api.launch.mock.calls[0][0]).toMatchObject({ model: "slow", speed: "standard", effort: null });
});

it("offers searchable model names and IDs on a long list", async () => {
  const offered = options();
  offered.models.push(...Array.from({ length: 14 }, (_, i) => ({ id: `extra-${i}`, label: `Choice ${i}`, efforts: [] })));
  api.options.mockResolvedValue(offered); await form();
  await wrapper.get("[data-testid=nt-model-search]").setValue("extra-13");
  expect(wrapper.findAll("#nt-task-model option").map(o => o.text())).toEqual(["Default", "Choice 13"]);
  await wrapper.get("#nt-task-model").setValue("extra-13");
  expect(wrapper.find("#nt-task-effort").exists()).toBe(false);
});

it("offers models and efforts on the other Agents and omits a separate speed", async () => {
  await form(); await wrapper.findAll(".nt-agent")[1].trigger("click"); await flushPromises();
  expect(wrapper.find(".nt-fast").exists()).toBe(false);
  await wrapper.get("#nt-task-model").setValue("fast"); await wrapper.get("#nt-task-effort").setValue("high");
  await start(); expect(api.launch.mock.calls[0][0]).toMatchObject({ kind: "claude", model: "fast", effort: "high" });
  expect(api.launch.mock.calls[0][0].speed).toBeUndefined();
});


it("retries a pre-upgrade request with its original receipt and inherited speed even when speed controls are now offered", async () => {
  const id = "10000000-0000-4000-8000-000000000001";
  localStorage.setItem("codex-desktop-pending-create", JSON.stringify({ key: JSON.stringify(["/project", "Task", "agent-default", null]), id }));
  api.launch.mockRejectedValue(new Error("historical unknown"));
  await form();
  expect(wrapper.get("[data-testid=nt-legacy-retry]").text()).toContain("original speed");
  await start(); await start();
  for (const [payload] of api.launch.mock.calls) {
    expect(payload.operationId).toBe(id); expect(payload.speed).toBeUndefined();
  }
});


it("keeps another Agent's unresolved create receipt when a different task succeeds", async () => {
  api.launch.mockRejectedValue(new Error("unknown")); await form(); await start();
  const receipt = localStorage.getItem("codex-desktop-pending-create");
  await wrapper.findAll(".nt-agent")[1].trigger("click"); await flushPromises();
  api.launch.mockResolvedValue(undefined); await start();
  expect(localStorage.getItem("codex-desktop-pending-create")).toBe(receipt);
});
