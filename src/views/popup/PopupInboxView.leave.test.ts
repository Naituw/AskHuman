import { defineComponent, h, onMounted, onBeforeUnmount } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInboxRequest } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";

// Leaving AskHuman while the expanded window shows only requests put away (docs/specs/popup-inbox-collapse.md C23, R11):
// after a short grace period the window shrinks back to the small window. The window is native; here the application's
// activation arrives as `popup-app-active` events and its starting state as the reply to `popup_app_active`.
const mock = vi.hoisted(() => ({
  invoke: vi.fn(), scopes: new Map<string, PopupScope>(),
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  mock.handlers.set(name, handler); return () => mock.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("../PopupView.vue", () => ({ default: defineComponent({ props: ["scope"], setup(props) {
  mock.scopes.set(props.scope.requestId, props.scope);
  onMounted(() => props.scope.ready(props.scope.requestId));
  onBeforeUnmount(() => mock.scopes.delete(props.scope.requestId));
  return () => h("div", [h("div", { class: "content" }, h("textarea", { "data-form-id": props.scope.requestId })), h("div", { class: "footer" }, "Send")]);
} }) }));
import PopupInboxView from "../PopupInboxView.vue";

function request(id: string): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project: "/project", source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Request ${id}`, predefinedOptions: [] }] } } };
}
function emit(name: string, payload: unknown) { mock.handlers.get(name)!({ payload }); }
interface LayoutArgs { sidebar: boolean; collapsed?: { width: number; height: number } }

describe("leaving AskHuman (C23, R11)", () => {
  let wrapper: VueWrapper;
  let ids: string[];
  let revision: number;
  // What native says about the application's activation when the page asks; a promise lets a test answer later.
  let nativeState: boolean | undefined | Promise<boolean> | Error;
  beforeEach(() => {
    ids = ["a", "b"]; revision = 0; nativeState = undefined;
    mock.handlers.clear(); mock.invoke.mockReset(); mock.scopes.clear();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.stubGlobal("CSS", { escape: (s: string) => s });
    mock.invoke.mockImplementation(async (command: string, args?: LayoutArgs) => {
      if (command === "popup_inbox_init") return { recovered: false, requests: ids.map(request) };
      if (command === "popup_app_active") { if (nativeState instanceof Error) throw nativeState; return nativeState; }
      if (command === "popup_inbox_layout") {
        return { revision: ++revision, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0, limited: false, collapsed: !!args?.collapsed };
      }
    });
  });
  afterEach(() => { wrapper?.unmount(); vi.useRealTimers(); vi.unstubAllGlobals(); });
  async function start(...initial: string[]) {
    if (initial.length) ids = initial;
    wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
  }
  const vm = () => wrapper.vm as unknown as { markLater(id: string): void };
  const tick = async (ms: number) => { await vi.advanceTimersByTimeAsync(ms); await flushPromises(); };
  // Putting a request away can jump to the next one, which swaps the body (126ms out, 174ms in).
  const later = async (id: string) => { vm().markLater(id); await flushPromises(); await tick(126 + 174); };
  const calls = (command: string) => mock.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const layouts = () => calls("popup_inbox_layout") as LayoutArgs[];
  const small = () => !!layouts()[layouts().length - 1]?.collapsed;
  const row = (index: number) => wrapper.findAll(".inbox-row")[index];
  const visible = (id: string) => wrapper.find(`[data-inbox-request="${id}"]`).isVisible();
  const marked = () => wrapper.findAll(".inbox-row.secondary").length;
  // Every request put away, then one of them opened from the small window: an expanded window with nothing waiting.
  async function peek() {
    await start("a", "b");
    await later("a"); await later("b");
    expect(small()).toBe(true);
    await row(0).trigger("click"); await flushPromises();
    expect(wrapper.find(".inbox-root").attributes("data-view")).toBe("expanded");
    expect(wrapper.find(".inbox-root").attributes("data-primary")).toBe("no");
    mock.invoke.mockClear();
  }

  describe("a peek", () => {
    it("shrinks back to the small window once the user has been away for the grace period, keeping marks and drafts", async () => {
      await peek();
      const editor = wrapper.find('[data-form-id="a"]');
      await editor.setValue("half-written answer");
      emit("popup-app-active", false);
      await tick(199);
      expect(layouts()).toEqual([]);
      await tick(1);
      expect(small()).toBe(true);
      expect(layouts()).toHaveLength(1);
      expect(marked()).toBe(2);
      await row(0).trigger("click"); await flushPromises();
      expect(small()).toBe(false);
      expect(wrapper.find('[data-form-id="a"]').element).toBe(editor.element);
      expect((editor.element as HTMLTextAreaElement).value).toBe("half-written answer");
    });

    it("stays open when the user is back before the grace period ends", async () => {
      await peek();
      emit("popup-app-active", false);
      await tick(100);
      emit("popup-app-active", true);
      await tick(500);
      expect(layouts()).toEqual([]);
      expect(visible("a")).toBe(true);
    });

    it("counts the grace period from the last time the user left", async () => {
      await peek();
      emit("popup-app-active", false);
      await tick(150);
      emit("popup-app-active", true);
      emit("popup-app-active", false);
      await tick(150);
      expect(layouts()).toEqual([]);
      await tick(50);
      expect(small()).toBe(true);
    });

    it("shrinks again after the user comes back and leaves once more", async () => {
      await peek();
      emit("popup-app-active", false);
      await tick(200);
      expect(small()).toBe(true);
      await row(0).trigger("click"); await flushPromises();
      expect(small()).toBe(false);
      emit("popup-app-active", true);
      await tick(10);
      emit("popup-app-active", false);
      await tick(200);
      expect(small()).toBe(true);
      expect(layouts()).toHaveLength(3);
    });

    it("does not shrink under the close confirmation", async () => {
      await peek();
      emit("popup-inbox-close", undefined); await flushPromises();
      emit("popup-app-active", false);
      await tick(500);
      expect(layouts().every(args => !args.collapsed)).toBe(true);
      expect(wrapper.find(".inbox-close-dialog").exists()).toBe(true);
    });

    it("lets a window change that is already under way finish first", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      let release!: () => void;
      const held = new Promise<void>(resolve => { release = resolve; });
      const normal = mock.invoke.getMockImplementation()!;
      mock.invoke.mockImplementation(async (command: string, args?: unknown) => {
        if (command === "popup_inbox_commit") await held;
        return normal(command, args);
      });
      mock.invoke.mockClear();
      // Opening a row starts the expansion, which waits for its commit; the user leaves meanwhile.
      await row(0).trigger("click"); await flushPromises();
      emit("popup-app-active", false);
      await tick(300);
      expect(layouts().every(args => !args.collapsed)).toBe(true);
      release(); await tick(0);
      // The expansion completed, and only then did the window shrink back.
      expect(layouts().map(args => !!args.collapsed)).toEqual([false, true]);
      expect(marked()).toBe(2);
    });

    it("drops the shrink when the user is back by the time it gets its turn", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      let release!: () => void;
      const held = new Promise<void>(resolve => { release = resolve; });
      const normal = mock.invoke.getMockImplementation()!;
      mock.invoke.mockImplementation(async (command: string, args?: unknown) => {
        if (command === "popup_inbox_commit") await held;
        return normal(command, args);
      });
      mock.invoke.mockClear();
      await row(0).trigger("click"); await flushPromises();
      emit("popup-app-active", false);
      await tick(300);
      emit("popup-app-active", true);
      release(); await tick(300);
      expect(layouts().map(args => !!args.collapsed)).toEqual([false]);
    });

    it("forgets a shrink that was waiting when the page goes away", async () => {
      await peek();
      emit("popup-app-active", false);
      await tick(100);
      wrapper.unmount();
      await tick(500);
      expect(layouts()).toEqual([]);
    });
  });

  describe("when the window does not need to shrink", () => {
    it("stays open while a request is waiting", async () => {
      await start("a", "b");
      await later("b");
      mock.invoke.mockClear();
      emit("popup-app-active", false);
      await tick(500);
      expect(layouts()).toEqual([]);
      expect(visible("a")).toBe(true);
    });

    it("leaves a window that is already small alone", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      mock.invoke.mockClear();
      emit("popup-app-active", false);
      await tick(500);
      expect(layouts()).toEqual([]);
    });
  });

  describe("the starting state", () => {
    // `b` is put away, and `a` ends from a remote channel: the user is taken to `b` only when AskHuman is the active app.
    async function remoteEnd() {
      await start("a", "b");
      await later("b");
      mock.invoke.mockClear();
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" }); await flushPromises();
      await tick(600);
    }

    it("asks native once the page listens, and only once", async () => {
      await start();
      expect(calls("popup_app_active")).toEqual([undefined]);
    });

    it("takes the app as active when native does not say", async () => {
      await remoteEnd();
      expect(calls("popup_inbox_activate")).toEqual([{ requestId: "b" }]);
      expect(small()).toBe(false);
    });

    it("knows a popup that was shown without activating the app: a remote end shrinks instead of jumping", async () => {
      nativeState = false;
      await remoteEnd();
      expect(small()).toBe(true);
      expect(calls("popup_inbox_activate")).toEqual([]);
    });

    it("does not shrink a peek because native answers late that the app was inactive at the start", async () => {
      // Only an event is the user leaving; the starting state says where the app stood when the page started.
      let answer!: (active: boolean) => void;
      nativeState = new Promise<boolean>(resolve => { answer = resolve; });
      await peek();
      answer(false); await tick(500);
      expect(layouts()).toEqual([]);
    });

    it("prefers an event that arrives before native's answer", async () => {
      let answer!: (active: boolean) => void;
      nativeState = new Promise<boolean>(resolve => { answer = resolve; });
      await start("a", "b");
      emit("popup-app-active", true);
      answer(false); await flushPromises();
      await later("b");
      mock.invoke.mockClear();
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" }); await flushPromises();
      await tick(600);
      expect(calls("popup_inbox_activate")).toEqual([{ requestId: "b" }]);
    });

    it("carries on when native cannot answer", async () => {
      nativeState = new Error("popup window is unavailable");
      await remoteEnd();
      expect(wrapper.find(".inbox-error").exists()).toBe(false);
      expect(calls("popup_inbox_activate")).toEqual([{ requestId: "b" }]);
    });
  });
});
