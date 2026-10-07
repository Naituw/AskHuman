import { defineComponent, h, onMounted, onBeforeUnmount } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInboxRequest } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";
const mock = vi.hoisted(() => ({ invoke: vi.fn(), nativeSync: vi.fn(async (_id?: string) => {}), heldReady: new Set<string>(), scopes: new Map<string, PopupScope>(), handlers: new Map<string, (event: { payload: unknown }) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  mock.handlers.set(name, handler); return () => mock.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: vi.fn() }) }));
vi.mock("../PopupView.vue", () => ({ default: defineComponent({ props: ["scope"], setup(props) {
  mock.scopes.set(props.scope.requestId, props.scope);
  onMounted(() => { if (!mock.heldReady.has(props.scope.requestId)) props.scope.ready(props.scope.requestId); });
  onMounted(() => props.scope.nativePreviewSync?.(() => mock.nativeSync(props.scope.requestId)));
  onBeforeUnmount(() => mock.scopes.delete(props.scope.requestId));
  return () => h("div", [h("div", { class: "content" }, h("textarea", { "data-form-id": props.scope.requestId })), h("div", { class: "footer" }, "Send")]);
} }) }));
import PopupInboxView from "../PopupInboxView.vue";
import UnreadRipple from "./UnreadRipple.vue";
function request(id: string, project = "/project"): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project, source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Request ${id}`, predefinedOptions: [] }] } } };
}
function emit(name: string, payload: unknown) { mock.handlers.get(name)!({ payload }); }
describe("shared popup navigation", () => {
  let wrapper: VueWrapper;
  let unmounted = false;
  beforeEach(() => {
    unmounted = false;
    mock.handlers.clear(); mock.invoke.mockReset(); mock.nativeSync.mockReset(); mock.nativeSync.mockResolvedValue();
    mock.heldReady.clear(); mock.scopes.clear();
    vi.stubGlobal("CSS", { escape: (s: string) => s });
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_inbox_init") return { recovered: false, requests: [request("a"), request("b")] };
      if (command === "popup_inbox_layout") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0 };
    });
  });
  afterEach(() => { if (!unmounted) wrapper?.unmount(); vi.useRealTimers(); vi.unstubAllGlobals(); });
  async function start() {
    wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
  }
  it("drags only the internal divider and persists the latest position after an in-flight update", async () => {
    await start();
    await wrapper.find('[data-form-id="a"]').setValue("retained draft");
    mock.invoke.mockClear();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrame = 0;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.set(++nextFrame, callback); return nextFrame; });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    const runFrame = () => { const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach(callback => callback(0)); };
    let release!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    mock.invoke.mockImplementation(async (command: string, args: { width: number; finished: boolean }) => {
      if (command === "popup_inbox_resize_sidebar") {
        if (!args.finished) await held;
        return { revision: 2, sidebarWidth: args.width, mainWidth: 800 - args.width, mainHeight: 620, previewWidth: 0 };
      }
    });
    const divider = wrapper.find(".inbox-divider");
    Object.defineProperty(divider.element, "setPointerCapture", { value: vi.fn(), configurable: true });
    const pointer = (type: string, clientX: number) => {
      const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX });
      Object.defineProperty(event, "pointerId", { value: 1 });
      divider.element.dispatchEvent(event);
      if (type === "pointerdown") expect(event.defaultPrevented).toBe(true);
    };
    pointer("pointerdown", 240);
    pointer("pointermove", 300); runFrame(); await flushPromises();
    expect(mock.invoke).toHaveBeenCalledWith("popup_inbox_resize_sidebar", { width: 300, finished: false });
    pointer("pointermove", 330);
    await divider.trigger("lostpointercapture"); await flushPromises();
    expect(mock.invoke.mock.calls).toHaveLength(1);
    release(); await flushPromises(); runFrame(); await flushPromises();
    expect(mock.invoke.mock.calls).toEqual([
      ["popup_inbox_resize_sidebar", { width: 300, finished: false }],
      ["popup_inbox_resize_sidebar", { width: 330, finished: true }],
    ]);
    expect(wrapper.classes()).not.toContain("inbox-frozen");
    expect((wrapper.find('[data-form-id="a"]').element as HTMLTextAreaElement).value).toBe("retained draft");
    expect((wrapper.element as HTMLElement).style.getPropertyValue("--inbox-sidebar")).toBe("330px");
  });
  it("keeps an arrival highlighted for all five flashes without clearing unread state", async () => {
    await start();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-show", request("c")); await flushPromises();
    const row = wrapper.findAll(".inbox-row")[2];
    expect(row.findComponent(UnreadRipple).props("active")).toBe(false);
    for (let cycle = 0; cycle < 4; cycle++) {
      await vi.advanceTimersByTimeAsync(400);
      expect(row.classes()).toContain("flash");
    }
    await vi.advanceTimersByTimeAsync(400);
    expect(row.classes()).not.toContain("flash");
    expect(row.find(".inbox-dot").classes()).toContain("unread");
    expect(row.findComponent(UnreadRipple).props("active")).toBe(true);
    expect(wrapper.find('[data-inbox-request="a"]').isVisible()).toBe(true);
    await row.trigger("click"); await flushPromises();
    expect(row.findComponent(UnreadRipple).props("active")).toBe(false);
    expect(row.classes()).not.toContain("unread");
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
  const phase = () => wrapper.attributes("data-completion-phase");
  async function tick(ms: number) { await vi.advanceTimersByTimeAsync(ms); await flushPromises(); }
  it("immediately moves the body after success and restores the successor's draft and selection", async () => {
    await start();
    await wrapper.findAll(".inbox-row")[1].trigger("click"); await flushPromises();
    const input = wrapper.find('[data-form-id="b"]').element as HTMLTextAreaElement;
    input.value = "saved answer"; input.focus(); input.setSelectionRange(2, 5);
    await wrapper.findAll(".inbox-row")[0].trigger("click"); await flushPromises();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    expect(phase()).toBe("out");
    expect(wrapper.find('[data-inbox-request="a"] .footer').text()).toBe("Send");
    expect(wrapper.text()).not.toContain("Sent");
    await wrapper.findAll(".inbox-row")[1].trigger("click"); await flushPromises();
    expect(wrapper.find('[data-inbox-request="a"]').isVisible()).toBe(true);
    expect(phase()).toBe("out");
    expect(wrapper.find('[data-inbox-request="a"]').attributes("data-answer-phase")).toBe("out");
    expect(wrapper.find(".inbox-entry-collapse").exists()).toBe(true);
    await tick(126); expect(phase()).toBe("in");
    expect(wrapper.find('[data-inbox-request="b"]').isVisible()).toBe(true);
    expect(wrapper.find('[data-inbox-request="b"] .footer').text()).toBe("Send");
    expect(mock.scopes.get("b")!.blocked.value).toBe(true);
    await tick(174);
    expect(phase()).toBeUndefined(); expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
    expect(mock.scopes.get("b")!.blocked.value).toBe(false);
    expect(input.value).toBe("saved answer"); expect(document.activeElement).toBe(input);
    expect([input.selectionStart, input.selectionEnd]).toEqual([2, 5]);
  });
  it("waits for an unopened form and native preview before clearing unread or revealing its body", async () => {
    await start(); mock.heldReady.add("b");
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    expect(wrapper.find('[data-inbox-request="b"]').isVisible()).toBe(false);
    await tick(600); expect(phase()).toBe("waiting");
    expect(wrapper.findAll(".inbox-row")[1].classes()).toContain("unread");
    mock.scopes.get("b")!.ready("b"); await flushPromises();
    expect(phase()).toBe("out");
    let release!: () => void;
    mock.nativeSync.mockImplementation(async id => { if (id === "b") {
      expect(mock.scopes.get("b")!.nativePreviewBlocked!.value).toBe(false);
      await new Promise<void>(resolve => { release = resolve; });
    } });
    await tick(126); expect(phase()).toBe("prepare");
    expect(wrapper.find('[data-inbox-request="b"]').attributes("data-answer-phase")).toBe("prepare");
    expect(wrapper.find('[data-inbox-request="b"]').classes()).toContain("inbox-preparing");
    expect(wrapper.find('[data-inbox-request="a"] .footer').text()).toBe("Send");
    expect(wrapper.findAll(".inbox-row")[0].classes()).toContain("selected");
    expect(wrapper.findAll(".inbox-row")[1].classes()).toContain("unread");
    release(); await flushPromises(); expect(phase()).toBe("in");
    expect(wrapper.findAll(".inbox-row")[1].classes()).not.toContain("unread");
    await tick(174); expect(phase()).toBeUndefined();
  });
  it("skips successors ended during preparation and incorporates arrivals while waiting for readiness", async () => {
    await start(); mock.heldReady.add("b");
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "submitted" }); await flushPromises();
    emit("popup-inbox-terminal", { requestId: "b", winner: "slack" });
    emit("popup-inbox-show", request("c", "/other")); await flushPromises();
    await tick(126); await tick(174);
    expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
    expect(wrapper.find('[data-inbox-request="c"]').isVisible()).toBe(true);
    expect(wrapper.find(".inbox-error").exists()).toBe(false);
    expect(mock.invoke.mock.calls.some(([cmd, args]) => cmd === "popup_inbox_activate" && args.requestId === "b")).toBe(false);
  });
  it("hides a successor ended during entrance and resumes with the next surviving request", async () => {
    await start(); emit("popup-inbox-show", request("c")); await flushPromises();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    await tick(126); expect(phase()).toBe("in");
    emit("popup-inbox-terminal", { requestId: "b", winner: "slack" }); await flushPromises();
    expect(wrapper.find('[data-inbox-request="b"]').isVisible()).toBe(false);
    await tick(174); expect(wrapper.find('[data-inbox-request="c"]').isVisible()).toBe(true);
    await tick(174); expect(phase()).toBeUndefined();
    expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
  });
  it("closes the last answered request immediately and accepts a subsequent arrival", async () => {
    mock.invoke.mockImplementation(async command => {
      if (command === "popup_inbox_init") return { requests: [request("a")], recovered: false };
      if (command === "popup_inbox_layout") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0 };
    });
    await start(); vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    expect(phase()).toBeUndefined(); expect(wrapper.findAll(".inbox-row")).toHaveLength(0);
    expect(mock.invoke.mock.calls.some(([cmd]) => cmd === "popup_inbox_idle")).toBe(true);
    emit("popup-inbox-show", request("b")); await flushPromises();
    expect(wrapper.find('[data-inbox-request="b"]').isVisible()).toBe(true);
  });
  it("switches without a confirmation or animation timer when reduced motion is enabled", async () => {
    vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
    await start(); vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "submitted" }); await flushPromises();
    expect(phase()).toBeUndefined();
    expect(wrapper.find('[data-inbox-request="b"]').isVisible()).toBe(true);
    expect(wrapper.find(".inbox-entry-collapse").exists()).toBe(false);
    emit("popup-inbox-terminal", { requestId: "b", winner: "popup", completion: "sent" }); await flushPromises();
    expect(wrapper.findAll(".inbox-row")).toHaveLength(0);
    expect(mock.invoke.mock.calls.some(([cmd]) => cmd === "popup_inbox_idle")).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });
  it("prepares a successor during sending while retaining the current form and unread state", async () => {
    await start();
    expect(wrapper.find('[data-inbox-request="b"]').exists()).toBe(false);
    mock.scopes.get("a")!.sending?.(); await flushPromises();
    expect(wrapper.find('[data-inbox-request="b"]').exists()).toBe(true);
    expect(wrapper.find('[data-inbox-request="b"]').isVisible()).toBe(false);
    expect(wrapper.find('[data-inbox-request="a"]').isVisible()).toBe(true);
    expect(wrapper.findAll(".inbox-row")[1].classes()).toContain("unread");
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    expect(phase()).toBe("out");
    await tick(126); await tick(174); expect(phase()).toBeUndefined();
  });
  it("never shows sent feedback for cancellation or an external winner", async () => {
    await start(); vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "slack", completion: "sent" }); await flushPromises();
    expect(phase()).toBeUndefined(); expect(wrapper.find(".inbox-completed-check").exists()).toBe(false);
    await tick(600);
    emit("popup-inbox-terminal", { requestId: "b", winner: "popup" }); await flushPromises();
    expect(wrapper.findAll(".inbox-row")).toHaveLength(0); expect(phase()).toBeUndefined();
  });
  it("settles all readiness and animation waits on disposal", async () => {
    await start(); mock.heldReady.add("b"); vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    wrapper.unmount(); unmounted = true; await flushPromises();
    const calls = mock.invoke.mock.calls.length;
    await tick(5000); expect(mock.invoke.mock.calls).toHaveLength(calls);
    expect(vi.getTimerCount()).toBe(0);
  });
  it("finishes a prepared native transaction without committing after disposal", async () => {
    await start(); vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    let release!: () => void;
    mock.nativeSync.mockImplementation(async id => { if (id === "b") await new Promise<void>(resolve => { release = resolve; }); });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    await tick(126); expect(phase()).toBe("prepare");
    wrapper.unmount(); unmounted = true;
    mock.invoke.mockClear(); release(); await flushPromises();
    expect(mock.invoke.mock.calls).toEqual([["popup_inbox_finish", { revision: 1, arrival: false }]]);
    expect(vi.getTimerCount()).toBe(0);
  });
  it("never reopens a committed answer when the successor's geometry fails", async () => {
    await start(); vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    mock.invoke.mockImplementation(async (command: string) => { if (command === "popup_inbox_layout") throw "layout unavailable"; });
    emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
    await tick(126);
    expect(wrapper.find('[data-inbox-request="a"]').exists()).toBe(false);
    expect(wrapper.find('[data-inbox-request="b"]').isVisible()).toBe(true);
    expect(wrapper.find(".inbox-error").text()).toBe("layout unavailable");
    expect(mock.scopes.get("b")!.blocked.value).toBe(false);
  });
});
