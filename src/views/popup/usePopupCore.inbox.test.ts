import { computed, defineComponent, h, ref, type Ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInit } from "../../lib/types";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), handlers: new Map<string, Set<(event: { payload: unknown }) => void>>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
  if (!mocks.handlers.has(name)) mocks.handlers.set(name, new Set());
  mocks.handlers.get(name)!.add(handler); return () => mocks.handlers.get(name)?.delete(handler);
}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import { usePopupCore, type PopupScope } from "./usePopupCore";
function init(id: string): PopupInit {
  return { theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project", language: "en", warm: true,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: "Choose", predefinedOptions: [{ text: "Yes", recommended: true }] }] } } };
}
describe("mounted inbox forms", () => {
  const wrappers: VueWrapper[] = [];
  beforeEach(() => {
    vi.clearAllMocks(); mocks.handlers.clear();
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    mocks.invoke.mockImplementation(async (command: string, args: { requestId?: string }) => {
      if (command === "popup_init") return init(args.requestId!);
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
  });
  afterEach(() => { wrappers.forEach(w => w.unmount()); wrappers.length = 0; vi.unstubAllGlobals(); });
  async function form(id: string, active: Ref<boolean>) {
    let ctx!: ReturnType<typeof usePopupCore>;
    const scope: PopupScope = { requestId: id, active, blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: vi.fn(),
      preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }) };
    const wrapper = mount(defineComponent({ setup() { ctx = usePopupCore(scope); return () => h("div"); } }), {
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper); await flushPromises(); return { ctx, scope };
  }
  it("routes the window shortcut to the active request and retains the hidden draft", async () => {
    const activeA = ref(true), activeB = ref(false);
    const a = await form("a", activeA), b = await form("b", activeB);
    a.ctx.inputByQ.value[0] = "draft A";
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1", ctrlKey: true }));
    expect(a.ctx.chosenByQ.value[0]).toEqual(["Yes"]); expect(b.ctx.chosenByQ.value[0]).toEqual([]);
    activeA.value = false; activeB.value = true; await flushPromises();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1", ctrlKey: true }));
    expect(b.ctx.chosenByQ.value[0]).toEqual(["Yes"]); expect(a.ctx.inputByQ.value[0]).toBe("draft A");
    await b.ctx.submit();
    expect(mocks.invoke).toHaveBeenCalledWith("submit_popup", expect.objectContaining({ requestId: "b" }));
  });
  it("keeps the draft and shows an acknowledgement failure without closing the request", async () => {
    const a = await form("a", ref(true)); a.ctx.inputByQ.value[0] = "unsent";
    mocks.invoke.mockImplementationOnce(async () => { throw new Error("connection unavailable"); });
    await a.ctx.submit();
    expect(a.ctx.inputByQ.value[0]).toBe("unsent"); expect(a.ctx.submitting.value).toBe(false);
    expect(a.ctx.submissionError.value).toContain("connection unavailable");
  });
});
