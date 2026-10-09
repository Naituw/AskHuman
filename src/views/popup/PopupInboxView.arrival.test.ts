import { defineComponent, h, nextTick, onMounted } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInboxRequest, PopupInboxSnapshot } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";
import { arrivalTiming } from "./inboxArrival";

const landedAt = arrivalTiming.appear + arrivalTiming.first + arrivalTiming.second + arrivalTiming.fly;

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), focus: vi.fn(), handlers: new Map<string, (e: { payload: unknown }) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, fn: (e: { payload: unknown }) => void) => {
  mocks.handlers.set(name, fn); return () => mocks.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: mocks.focus }) }));
vi.mock("../PopupView.vue", () => ({ default: defineComponent({ props: ["scope"], setup(props) {
  onMounted(() => (props.scope as PopupScope).ready((props.scope as PopupScope).requestId));
  return () => h("div", { class: "popup-main" }, [h("textarea", { "data-editor": (props.scope as PopupScope).requestId })]);
} }) }));
import PopupInboxView from "../PopupInboxView.vue";

function request(id: string, sequence: number): PopupInboxRequest {
  return { requestId: id, sequence, project: "/project", source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: id, files: [] }, questions: [] } } };
}
function rect(left: number, top: number, width: number, height: number) {
  return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON: () => ({}) } as DOMRect;
}
describe("inbox arrival lifecycle", () => {
  let wrapper: VueWrapper | undefined, now: number, frameId: number, mainWidth: number, hidden: boolean, reduced: boolean;
  let snapshot: PopupInboxSnapshot;
  const frames = new Map<number, FrameRequestCallback>();
  beforeEach(() => {
    vi.clearAllMocks(); mocks.handlers.clear(); frames.clear();
    now = 0; frameId = 0; mainWidth = 560; hidden = false; reduced = false;
    snapshot = { requests: [request("a", 1)], recovered: false, presented: true };
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.spyOn(performance, "now").mockImplementation(() => now);
    vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
    vi.stubGlobal("CSS", { escape: (id: string) => id });
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { frames.set(++frameId, cb); return frameId; });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    vi.stubGlobal("matchMedia", () => ({ get matches() { return reduced; }, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("inbox-root")) return rect(0, 0, 1600, 620);
      if (this.classList.contains("popup-main")) return rect(606, 0, mainWidth, 620);
      if (this.classList.contains("inbox-sidebar")) return rect(360, 0, 240, 620);
      if (this.classList.contains("inbox-navigation")) return rect(360, 70, 240, 520);
      const row = this.closest<HTMLElement>(".inbox-row");
      if (row) {
        const index = [...row.parentElement!.parentElement!.querySelectorAll(".inbox-row")].indexOf(row);
        const scroll = row.closest(".inbox-navigation")!.scrollTop;
        if (this.classList.contains("inbox-dot")) return rect(378, 115 + index * 72 - scroll, 6, 6);
        return rect(368, 100 + index * 72 - scroll, 224, 72);
      }
      return rect(0, 0, 560, 620);
    });
    mocks.invoke.mockImplementation(async (command: string, args?: { sidebar?: boolean }) => {
      if (command === "popup_inbox_init") return snapshot;
      if (command === "popup_inbox_layout" || command === "popup_inbox_finish") return { revision: 1, mainWidth, mainHeight: 620,
        sidebarWidth: command === "popup_inbox_finish" ? 240 : args?.sidebar ? 240 : 0, previewWidth: 0, limited: false,
        frame: { x: 0, y: 0, width: 806, height: 620 }, canvas: { left: 606, frozen: false } };
    });
  });
  afterEach(() => { wrapper?.unmount(); wrapper = undefined; document.body.innerHTML = ""; vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
  async function open() {
    wrapper = mount(PopupInboxView, { attachTo: document.body, global: {
      plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })], stubs: { UnreadRipple: true },
    } });
    await flushPromises(); return wrapper;
  }
  async function emit(name: string, payload?: unknown) { mocks.handlers.get(name)!({ payload }); await flushPromises(); }
  async function step(time: number) {
    now = time; const batch = [...frames.values()]; frames.clear(); batch.forEach(cb => cb(time)); await nextTick();
  }
  it("keeps the current editor, selection and composition focus while two notices overlap", async () => {
    const view = await open();
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
    const editor = view.get("textarea").element as HTMLTextAreaElement;
    editor.value = "draftabcdef"; editor.focus(); editor.setSelectionRange(3, 8);
    editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    await emit("popup-inbox-show", request("b", 2)); await step(400);
    await emit("popup-inbox-show", request("c", 3)); await step(800);
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(2);
    expect(document.activeElement).toBe(editor); expect(editor.value).toBe("draftabcdef");
    expect([editor.selectionStart, editor.selectionEnd]).toEqual([3, 8]); expect(mocks.focus).not.toHaveBeenCalled();
    expect(view.get('[data-inbox-row="b"] .inbox-dot').classes()).not.toContain("unread");
    await step(landedAt); expect(view.findAll(".inbox-arrival-floater")).toHaveLength(1);
    expect(view.get('[data-inbox-row="b"] .inbox-dot').classes()).toContain("unread");
    expect(view.get('[data-inbox-row="b"]').classes()).toContain("flash");
    await step(landedAt + 400); expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
  });
  it("uses the answer and sidebar bounds, excluding a native preview and offscreen canvas reserve", async () => {
    mainWidth = 400; const view = await open(); await emit("popup-inbox-show", request("b", 2)); await step(220);
    expect(view.get(".inbox-arrival-floater").attributes("style")).toContain("translate3d(639px, 256px, 0)");
  });
  it("waits for first native presentation and cancels terminal requests before queued removal", async () => {
    snapshot.presented = false; const view = await open(); await emit("popup-inbox-show", request("b", 2));
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
    await emit("popup-inbox-presented"); expect(view.findAll(".inbox-arrival-floater")).toHaveLength(1);
    await emit("popup-inbox-terminal", { requestId: "b", winner: "slack" }); await step(2000);
    expect(view.find('[data-inbox-row="b"]').exists()).toBe(false);
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0); expect(frames.size).toBe(0);
  });
  it("plays a cold-start burst after presentation without animating its first request", async () => {
    snapshot.presented = false; snapshot.requests.push(request("b", 2));
    const view = await open(); expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
    await emit("popup-inbox-presented");
    expect(view.findAll(".inbox-arrival-floater").map(item => item.attributes("data-arrival-id"))).toEqual(["b"]);
  });
  it("does not replay recovered requests but still animates later arrivals", async () => {
    snapshot.recovered = true; snapshot.presented = false; snapshot.requests.push(request("b", 2));
    const view = await open(); await emit("popup-inbox-presented");
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
    await emit("popup-inbox-show", request("c", 3));
    expect(view.findAll(".inbox-arrival-floater").map(item => item.attributes("data-arrival-id"))).toEqual(["c"]);
  });
  it("does not resurrect unread state when the flying request is opened", async () => {
    const view = await open(); await emit("popup-inbox-show", request("b", 2)); await step(500);
    await view.get('[data-inbox-row="b"]').trigger("click"); await flushPromises(); await step(2000);
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
    expect(view.get('[data-inbox-row="b"] .inbox-dot').classes()).not.toContain("unread");
    expect(view.get('[data-inbox-row="b"]').classes()).not.toContain("flash");
  });
  it("scrolls only navigation once and lets user scrolling end an offscreen flight", async () => {
    snapshot.requests.push(...Array.from({ length: 7 }, (_, i) => request(`old${i}`, i + 2)));
    const view = await open(); await emit("popup-inbox-show", request("b", 20));
    const nav = view.get(".inbox-navigation").element;
    expect(nav.scrollTop).toBeGreaterThan(0); const initial = nav.scrollTop; await step(300);
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(1);
    nav.scrollTop = 0; await step(400);
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
    expect(nav.scrollTop).toBe(0); expect(initial).toBeGreaterThan(0);
    expect(view.get('[data-inbox-row="b"] .inbox-dot').classes()).toContain("unread");
  });
  it("keeps static unread indicators with reduced motion and releases hidden/unmounted frames", async () => {
    reduced = true; const view = await open(); await emit("popup-inbox-show", request("b", 2));
    expect(view.findAll(".inbox-arrival-floater")).toHaveLength(0);
    expect(view.get('[data-inbox-row="b"] .inbox-dot').classes()).toContain("unread");
    reduced = false; await emit("popup-inbox-show", request("c", 3)); expect(frames.size).toBe(1);
    hidden = true; document.dispatchEvent(new Event("visibilitychange")); await nextTick(); expect(frames.size).toBe(0);
    hidden = false; await emit("popup-inbox-show", request("d", 4)); view.unmount(); wrapper = undefined;
    expect(frames.size).toBe(0);
  });
});
