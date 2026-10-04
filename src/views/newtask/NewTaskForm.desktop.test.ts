import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import NewTaskForm from "./NewTaskForm.vue";
import { i18n } from "../../i18n";
const api = vi.hoisted(() => ({launch:vi.fn(),status:vi.fn()}));
vi.mock("@tauri-apps/api/event", () => ({listen: vi.fn(async () => vi.fn())}));
vi.mock("../../lib/ipc", () => ({
  agentTaskReadiness: vi.fn(async () => [{kind:"codex",label:"Codex",ready:false,binaryReady:false,integrationReady:true,lifecycleReady:false,integrationMode:"mcp"}]),
  codexLaunchStatus: api.status,
  newTaskInit: vi.fn(async () => ({popupSubmitKey:"cmdEnter",permissionPrompt:"agent-default"})),
  newTaskLaunch: api.launch,
  newTaskProjects: vi.fn(async () => []), newTaskProjectsRefreshed:vi.fn(async () => []),
  openPath:vi.fn(),openSettings:vi.fn(), projectKeyOf: vi.fn(async () => "/project"), todosList: vi.fn(async () => []),
}));
let wrapper: VueWrapper;
beforeEach(() => {
  localStorage.clear();
  api.status.mockReset().mockResolvedValue({preference:"desktop",target:"desktop",desktopAvailable:true,terminalAvailable:false,integrated:true});
  api.launch.mockReset().mockResolvedValue(undefined);
  i18n.global.locale.value = "en";
});
afterEach(() => wrapper?.unmount());
async function form() {
  wrapper = mount(NewTaskForm,{props:{lockProject:"/project"},global:{plugins:[i18n]}});
  await flushPromises(); await wrapper.get(".nt-agent").trigger("click"); await wrapper.get("textarea").setValue("Task");
}
it("allows an App-only installation without a runtime selection step", async () => {
  await form();
  expect(wrapper.get(".nt-agent").classes()).not.toContain("disabled");
  expect(wrapper.get(".nt-agent").text()).toContain("New tasks will use Desktop App");
  expect(wrapper.find("#nt-target").exists()).toBe(false);
  await wrapper.get(".nt-btn-launch").trigger("click"); await flushPromises();
  expect(api.launch).toHaveBeenCalledTimes(1);
  expect(api.launch.mock.calls[0][0]).toMatchObject({kind:"codex",workspace:"/project",task:"Task"});
  expect(api.launch.mock.calls[0][0].launchTarget).toBeUndefined();
});
it("retains the operation ID across failed launches and a preference change", async () => {
  api.launch.mockRejectedValue(new Error("unknown outcome")); await form();
  await wrapper.get(".nt-btn-launch").trigger("click"); await flushPromises();
  const first = api.launch.mock.calls[0][0].operationId;
  api.status.mockResolvedValue({preference:"terminal",target:"terminal",desktopAvailable:true,terminalAvailable:true,integrated:true});
  window.dispatchEvent(new Event("focus")); await flushPromises();
  await wrapper.get(".nt-btn-launch").trigger("click"); await flushPromises();
  expect(api.launch.mock.calls[1][0].operationId).toBe(first);
  expect(wrapper.emitted("launched")).toBeUndefined();
});
