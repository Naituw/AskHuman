import { defineComponent, h, onMounted } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInboxRequest } from "../../lib/types";
const mock = vi.hoisted(() => ({ invoke: vi.fn(), nativeSync: vi.fn(async () => {}), handlers: new Map<string, (event: { payload: unknown }) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  mock.handlers.set(name, handler); return () => mock.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: vi.fn() }) }));
vi.mock("../PopupView.vue", () => ({ default: defineComponent({ props: ["scope"], setup(props) {
  onMounted(() => props.scope.ready(props.scope.requestId));
  onMounted(() => props.scope.nativePreviewSync?.(mock.nativeSync));
  return () => h("textarea", { "data-form-id": props.scope.requestId });
} }) }));
import PopupInboxView from "../PopupInboxView.vue";
function request(id: string, project = "/project"): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project, source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Request ${id}`, predefinedOptions: [] }] } } };
}
function emit(name: string, payload: unknown) { mock.handlers.get(name)!({ payload }); }
describe("shared popup navigation", () => {
  let wrapper: VueWrapper;
  beforeEach(() => {
    mock.handlers.clear(); mock.invoke.mockReset(); mock.nativeSync.mockReset(); mock.nativeSync.mockResolvedValue();
    vi.stubGlobal("CSS", { escape: (s: string) => s });
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_inbox_init") return { recovered: false, requests: [request("a"), request("b")] };
      if (command === "popup_inbox_layout") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0 };
    });
  });
  afterEach(() => { wrapper?.unmount(); vi.useRealTimers(); vi.unstubAllGlobals(); });
  async function start() {
    wrapper = mount(PopupInboxView, { global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
  }
  it("keeps an arrival highlighted for all three flashes without clearing unread state", async () => {
    await start();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-show", request("c")); await flushPromises();
    const row = wrapper.findAll(".inbox-row")[2];
    await vi.advanceTimersByTimeAsync(800);
    expect(row.classes()).toContain("flash");
    await vi.advanceTimersByTimeAsync(800);
    expect(row.classes()).toContain("flash");
    await vi.advanceTimersByTimeAsync(800);
    expect(row.classes()).not.toContain("flash");
    expect(row.find(".inbox-dot").classes()).toContain("unread");
    expect(wrapper.find('[data-inbox-request="a"]').isVisible()).toBe(true);
  });
  it("skips a successor already ended by a batch and opens the surviving arrival", async () => {
    await start(); emit("popup-inbox-show", request("c", "/other")); await flushPromises();
    mock.invoke.mockImplementation(async (command: string, args: { requestId?: string }) => {
      if (command === "popup_inbox_activate" && args.requestId === "b") throw "popup request is no longer pending";
      if (command === "popup_inbox_layout") return { revision: 2, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0 };
    });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup" });
    emit("popup-inbox-terminal", { requestId: "b", winner: "popup" });
    await flushPromises();
    expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
    expect(wrapper.find('[data-inbox-request="c"]').isVisible()).toBe(true);
    expect(wrapper.find(".inbox-error").exists()).toBe(false);
  });
  it("retains mounted drafts and keeps arrivals out of the close snapshot", async () => {
    await start(); await wrapper.find('[data-form-id="a"]').setValue("draft a");
    await wrapper.findAll(".inbox-row")[1].trigger("click"); await flushPromises();
    await wrapper.findAll(".inbox-row")[0].trigger("click"); await flushPromises();
    expect((wrapper.find('[data-form-id="a"]').element as HTMLTextAreaElement).value).toBe("draft a");
    emit("popup-inbox-close", undefined); await flushPromises();
    emit("popup-inbox-show", request("c")); await flushPromises();
    await wrapper.find(".inbox-cancel").trigger("click"); await flushPromises();
    expect(mock.invoke.mock.calls.filter(([cmd]) => cmd === "cancel_popup").map(([, args]) => args.requestId)).toEqual(["a", "b"]);
  });

  it("keeps main canvas coordinates fixed while native commit is pending", async () => {
    let release!: () => void;
    const frame = new Promise<void>(resolve => { release = resolve; });
    let sidebarFrame = false;
    let revision = 0;
    mock.invoke.mockImplementation(async (command: string, args: { sidebar?: boolean }) => {
      if (command === "popup_inbox_init") return { recovered: false, requests: [request("a")] };
      if (command === "popup_inbox_layout") {
        sidebarFrame = !!args.sidebar;
        return { revision: ++revision, sidebarWidth: args.sidebar ? 240 : 0, mainWidth: 560,
          mainHeight: 620, previewWidth: 0, canvas: { left: 606, width: 2766, height: 620 }, frame: { width: args.sidebar ? 806 : 560 } };
      }
      if (command === "popup_inbox_commit" && sidebarFrame) return frame;
    });
    await start();
    const before = (wrapper.element as HTMLElement).style.getPropertyValue("--inbox-canvas-left");
    mock.invoke.mockClear(); emit("popup-inbox-show", request("b")); await flushPromises();
    expect(wrapper.classes()).toContain("inbox-native");
    expect((wrapper.element as HTMLElement).style.getPropertyValue("--inbox-canvas-left")).toBe(before);
    expect((wrapper.element as HTMLElement).style.getPropertyValue("--inbox-main")).toBe("560px");
    expect(mock.invoke.mock.calls.some(([cmd]) => cmd === "popup_inbox_finish")).toBe(false);
    release(); await flushPromises();
    expect(mock.invoke.mock.calls.some(([cmd]) => cmd === "popup_inbox_finish")).toBe(true);
  });
  it("places the native preview before the window reveals it", async () => {
    await start(); mock.invoke.mockClear();
    let release!: () => void;
    mock.nativeSync.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; }));
    emit("popup-inbox-show", request("c")); await flushPromises();
    expect(mock.invoke.mock.calls.some(([cmd]) => cmd === "popup_inbox_commit")).toBe(false);
    release(); await flushPromises();
    expect(mock.invoke.mock.calls.some(([cmd]) => cmd === "popup_inbox_finish")).toBe(true);
    const commitIndex = mock.invoke.mock.calls.findIndex(([cmd]) => cmd === "popup_inbox_commit");
    expect(mock.nativeSync.mock.invocationCallOrder[mock.nativeSync.mock.invocationCallOrder.length - 1]).toBeLessThan(mock.invoke.mock.invocationCallOrder[commitIndex]);
  });
  it("pins the used body before native preparation and resumes only after finish", async () => {
    let release!: () => void;
    const finish = new Promise<void>(resolve => { release = resolve; });
    const normal = { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0,
      canvas: { left: 606, width: 1166, height: 620, frozen: false }, frame: { width: 806, height: 620 } };
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_inbox_init") return { requests: [request("a")], recovered: false };
      if (command === "popup_inbox_layout") return { ...normal, canvas: { ...normal.canvas, width: 4000, frozen: true } };
      if (command === "popup_inbox_finish") return normal;
    });
    await start();
    expect(wrapper.classes()).not.toContain("inbox-frozen");
    const measure = vi.spyOn(wrapper.find(".inbox-body").element, "getBoundingClientRect");
    measure.mockReturnValue({ width: 720 } as DOMRect);
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_inbox_layout") {
        expect(wrapper.classes()).toContain("inbox-frozen");
        expect((wrapper.element as HTMLElement).style.getPropertyValue("--inbox-body-width")).toBe("720px");
        expect(measure).toHaveBeenCalled();
        return { ...normal, canvas: { ...normal.canvas, width: 4000, frozen: true } };
      }
      if (command === "popup_inbox_finish") { await finish; return normal; }
    });
    emit("popup-inbox-show", request("b")); await flushPromises();
    expect(wrapper.classes()).toContain("inbox-frozen");
    release(); await flushPromises();
    expect(wrapper.classes()).not.toContain("inbox-frozen");
    expect((wrapper.element as HTMLElement).style.getPropertyValue("--inbox-main")).toContain("100vw");
    measure.mockRestore();
  });
  it("rejects stale replay after a terminal and idles only when all requests ended", async () => {
    await start(); emit("popup-inbox-terminal", { requestId: "b", winner: "popup" }); await flushPromises();
    emit("popup-inbox-show", request("b")); await flushPromises();
    expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
    expect(mock.invoke).not.toHaveBeenCalledWith("popup_inbox_idle", undefined);
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup" }); await flushPromises();
    expect(wrapper.findAll(".inbox-row")).toHaveLength(0);
    expect(mock.invoke.mock.calls.some(([cmd]) => cmd === "popup_inbox_idle")).toBe(true);
  });
});
