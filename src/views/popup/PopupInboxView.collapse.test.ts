import { defineComponent, h, onMounted, onBeforeUnmount } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInboxRequest } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";

// "Answer later" flows of the shared popup page: marks, the small window form, and what each event does to it
// (docs/specs/popup-inbox-collapse.md R1–R12). The window is native; here it is the `popup_inbox_layout` calls.
const mock = vi.hoisted(() => ({
  invoke: vi.fn(), focus: vi.fn(), heldReady: new Set<string>(), scopes: new Map<string, PopupScope>(),
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  mock.handlers.set(name, handler); return () => mock.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: mock.focus }) }));
vi.mock("../PopupView.vue", () => ({ default: defineComponent({ props: ["scope"], setup(props) {
  mock.scopes.set(props.scope.requestId, props.scope);
  onMounted(() => { if (!mock.heldReady.has(props.scope.requestId)) props.scope.ready(props.scope.requestId); });
  onBeforeUnmount(() => mock.scopes.delete(props.scope.requestId));
  return () => h("div", [h("div", { class: "content" }, h("textarea", { "data-form-id": props.scope.requestId })), h("div", { class: "footer" }, "Send")]);
} }) }));
import PopupInboxView from "../PopupInboxView.vue";

function request(id: string, project = "/project"): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project, source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Request ${id}`, predefinedOptions: [] }] } } };
}
function emit(name: string, payload: unknown) { mock.handlers.get(name)!({ payload }); }
interface LayoutArgs { sidebar: boolean; collapsed?: { width: number; height: number } }
const SHRINK_REFUSED = "The window cannot shrink while it is maximized or full screen";

describe("answer later and the small window", () => {
  let wrapper: VueWrapper;
  let ids: string[];
  let revision: number;
  let refuse: string | null;
  beforeEach(() => {
    ids = ["a", "b"]; revision = 0; refuse = null;
    mock.handlers.clear(); mock.invoke.mockReset(); mock.focus.mockReset(); mock.heldReady.clear(); mock.scopes.clear();
    vi.stubGlobal("CSS", { escape: (s: string) => s });
    mock.invoke.mockImplementation(async (command: string, args?: LayoutArgs) => {
      if (command === "popup_inbox_init") return { recovered: false, requests: ids.map(id => request(id)) };
      if (command === "popup_inbox_layout") {
        if (args?.collapsed && refuse) throw refuse;
        return { revision: ++revision, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0, limited: false,
          collapsed: !!args?.collapsed };
      }
    });
  });
  afterEach(() => { wrapper?.unmount(); vi.useRealTimers(); vi.unstubAllGlobals(); });
  async function start(...initial: string[]) {
    if (initial.length) ids = initial;
    wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
    mock.invoke.mockClear();
  }
  const scope = (id: string) => mock.scopes.get(id)!;
  // The footer button reaches the page through the request's scope; the Sidebar row icon (next phase) reaches it
  // by id, which is also how a request that has never been opened is marked.
  const page = () => wrapper.vm as unknown as { markLater(id: string): void; unmarkLater(id: string): void };
  const later = async (id: string) => { (mock.scopes.get(id)?.markLater ?? (() => page().markLater(id)))(); await flushPromises(); };
  const unlater = async (id: string) => { (mock.scopes.get(id)?.unmarkLater ?? (() => page().unmarkLater(id)))(); await flushPromises(); };
  const layouts = () => mock.invoke.mock.calls.filter(([command]) => command === "popup_inbox_layout").map(([, args]) => args as LayoutArgs);
  const calls = (command: string) => mock.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const activated = () => calls("popup_inbox_activate").map(args => args.requestId);
  const lastLayout = () => layouts()[layouts().length - 1];
  const small = () => !!lastLayout()?.collapsed;
  const visible = (id: string) => wrapper.find(`[data-inbox-request="${id}"]`).isVisible();
  // jsdom has no `inert` property, so Vue renders the attribute as the string "true" / "false".
  const bodyInert = () => { const value = wrapper.find(".inbox-body").attributes("inert"); return value !== undefined && value !== "false"; };
  async function tick(ms: number) { await vi.advanceTimersByTimeAsync(ms); await flushPromises(); }

  describe("marking (R1–R3)", () => {
    it("collapses a lone request to the small window without answering or cancelling it", async () => {
      await start("a");
      await later("a");
      const size = lastLayout().collapsed!;
      expect(size.width).toBeGreaterThanOrEqual(1);
      expect(size.height).toBeGreaterThanOrEqual(1);
      expect(scope("a").secondary!.value).toBe(true);
      expect(calls("cancel_popup")).toEqual([]);
      expect(calls("popup_inbox_idle")).toEqual([]);
      expect(wrapper.find('[data-inbox-request="a"]').exists()).toBe(true);
      expect(bodyInert()).toBe(true);
      expect(scope("a").nativePreviewBlocked!.value).toBe(true);
    });

    it("continues with the next primary request and keeps the first one marked", async () => {
      await start("a", "b");
      await later("a");
      expect(layouts().every(args => !args.collapsed)).toBe(true);
      expect(activated()).toEqual(["b"]);
      expect(visible("b")).toBe(true);
      expect(scope("a").secondary!.value).toBe(true);
      expect(scope("b").secondary!.value).toBe(false);
    });

    it("marks another row without touching the window or the open request, then skips marked ones and wraps", async () => {
      await start("a", "b", "c");
      await later("b");
      expect(layouts()).toEqual([]);
      expect(calls("popup_inbox_activate")).toEqual([]);
      expect(scope("b")).toBeUndefined();
      await later("a");
      expect(activated()).toEqual(["c"]);
      expect(visible("c")).toBe(true);
      expect(small()).toBe(false);
      await later("c");
      expect(small()).toBe(true);
    });

    it("keeps a marked request unread until its body is opened", async () => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      await start("a", "b");
      await later("b");
      const row = () => wrapper.findAll(".inbox-row")[1];
      expect(row().classes()).toContain("unread");
      await row().trigger("click"); await tick(126);
      expect(row().classes()).not.toContain("unread");
      expect(scope("b").secondary!.value).toBe(true);
    });

    it("keeps drafts: putting a request away and bringing it back leaves its form untouched", async () => {
      await start("a");
      const editor = wrapper.find('[data-form-id="a"]');
      await editor.setValue("half-written answer");
      await later("a");
      expect(small()).toBe(true);
      await unlater("a");
      expect(wrapper.find('[data-form-id="a"]').element).toBe(editor.element);
      expect((editor.element as HTMLTextAreaElement).value).toBe("half-written answer");
    });

    it("restoring a mark while expanded only flips the mark", async () => {
      await start("a", "b");
      await later("a");
      mock.invoke.mockClear();
      await unlater("a");
      expect(scope("a").secondary!.value).toBe(false);
      expect(mock.invoke).not.toHaveBeenCalled();
    });

    it("ignores marking while the close dialog blocks the page", async () => {
      await start("a", "b");
      emit("popup-inbox-close", undefined); await flushPromises();
      await later("a");
      expect(scope("a").secondary!.value).toBe(false);
      expect(layouts()).toEqual([]);
    });
  });

  describe("leaving the small window (R2, R6, C9)", () => {
    async function allMarked() {
      await start("a", "b");
      await later("a"); await later("b");
      expect(small()).toBe(true);
    }

    it("expands onto a request that is restored to primary", async () => {
      await allMarked();
      await unlater("b");
      expect(small()).toBe(false);
      expect(scope("b").secondary!.value).toBe(false);
      expect(visible("b")).toBe(true);
      expect(bodyInert()).toBe(false);
      expect(scope("b").nativePreviewBlocked!.value).toBe(false);
    });

    it("expands onto a row that is opened, keeping its mark", async () => {
      await allMarked();
      await wrapper.findAll(".inbox-row")[0].trigger("click"); await flushPromises();
      expect(small()).toBe(false);
      expect(visible("a")).toBe(true);
      expect(scope("a").secondary!.value).toBe(true);
      expect(scope("b").secondary!.value).toBe(true);
    });

    it("expands and focuses the window when the tray opens a request", async () => {
      await allMarked();
      emit("popup-inbox-focus", "a"); await flushPromises();
      expect(small()).toBe(false);
      expect(visible("a")).toBe(true);
      expect(scope("a").secondary!.value).toBe(true);
      expect(mock.focus).toHaveBeenCalled();
    });

    it("restores the focus the body had before it was put away", async () => {
      await start("a");
      const editor = wrapper.find('[data-form-id="a"]').element as HTMLTextAreaElement;
      editor.focus();
      expect(document.activeElement).toBe(editor);
      await later("a");
      expect(document.activeElement).not.toBe(editor);
      await unlater("a");
      expect(document.activeElement).toBe(editor);
    });
  });

  describe("close confirmation", () => {
    it("expands the small window onto the request it would open before showing the confirmation", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      expect(small()).toBe(true);
      emit("popup-inbox-close", undefined); await flushPromises();
      expect(small()).toBe(false);
      expect(wrapper.find(".inbox-close-dialog").exists()).toBe(true);
      expect(visible("b")).toBe(true);
      expect(scope("b").secondary!.value).toBe(true);
      // The dialog is usable: the user can continue answering.
      await wrapper.find(".inbox-continue").trigger("click"); await flushPromises();
      expect(wrapper.find(".inbox-close-dialog").exists()).toBe(false);
    });

    it("keeps the window open for a marking that was queued behind the confirmation", async () => {
      await start("a");
      // The marking is queued; the confirmation opens before the queue gets to it.
      scope("a").markLater!();
      emit("popup-inbox-close", undefined); await flushPromises();
      expect(wrapper.find(".inbox-close-dialog").exists()).toBe(true);
      expect(layouts().every(args => !args.collapsed)).toBe(true);
      expect(bodyInert()).toBe(true); // the dialog blocks the page, not the small window
    });
  });

  describe("arrival (R5, C4, C19)", () => {
    it("opens the small window onto a new request, which is primary, after it can be answered", async () => {
      await start("a");
      await later("a");
      mock.heldReady.add("c");
      mock.invoke.mockClear();
      emit("popup-inbox-show", request("c")); await flushPromises();
      expect(layouts()).toEqual([]);
      expect(wrapper.find('[data-inbox-request="c"]').exists()).toBe(true);
      expect(visible("c")).toBe(false);
      scope("c").ready("c"); await flushPromises();
      expect(small()).toBe(false);
      expect(visible("c")).toBe(true);
      expect(scope("c").secondary!.value).toBe(false);
      expect(scope("a").secondary!.value).toBe(true);
      expect(calls("popup_inbox_finish").map(args => args.arrival)).toEqual([true]);
      expect(wrapper.find(".inbox-error").exists()).toBe(false);
    });

    it("keeps the body when the window is already expanded", async () => {
      await start("a");
      emit("popup-inbox-show", request("b")); await flushPromises();
      expect(visible("a")).toBe(true);
      expect(layouts().every(args => !args.collapsed)).toBe(true);
    });

    it("leaves the window small when the new request ends before it can be shown", async () => {
      await start("a");
      await later("a");
      mock.heldReady.add("c");
      mock.invoke.mockClear();
      emit("popup-inbox-show", request("c")); await flushPromises();
      emit("popup-inbox-terminal", { requestId: "c", winner: "slack" }); await flushPromises();
      expect(layouts().every(args => !!args.collapsed)).toBe(true);
      expect(small()).toBe(true);
      expect(bodyInert()).toBe(true);
      expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
      expect(wrapper.find(".inbox-error").exists()).toBe(false);
    });
  });

  describe("an open request ends (R4, R6, C22)", () => {
    beforeEach(() => { vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] }); });

    it("takes a user who just answered to the next secondary request and keeps the window open", async () => {
      await start("a", "b");
      await later("b");
      emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "sent" }); await flushPromises();
      await tick(126); await tick(174);
      expect(activated()).toEqual(["b"]);
      expect(visible("b")).toBe(true);
      expect(small()).toBe(false);
      expect(scope("b").secondary!.value).toBe(true);
    });

    it("prefers the next primary request after an answer", async () => {
      await start("a", "b", "c");
      await later("b");
      emit("popup-inbox-terminal", { requestId: "a", winner: "popup", completion: "submitted" }); await flushPromises();
      await tick(126); await tick(174);
      expect(activated()).toEqual(["c"]);
      expect(visible("c")).toBe(true);
    });

    it("shrinks instead of jumping when a remote channel ends it while nobody is looking", async () => {
      await start("a", "b");
      await later("b");
      emit("popup-app-active", false);
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" }); await flushPromises();
      await tick(600);
      expect(small()).toBe(true);
      expect(calls("popup_inbox_activate")).toEqual([]);
      expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
      expect(bodyInert()).toBe(true);
      // Opening the remaining request from the small window works as usual.
      await wrapper.findAll(".inbox-row")[0].trigger("click"); await flushPromises();
      expect(small()).toBe(false);
      expect(visible("b")).toBe(true);
      expect(scope("b").secondary!.value).toBe(true);
    });

    it("does not shrink under an open close confirmation even when the app is not active", async () => {
      await start("a", "b");
      await later("b");
      emit("popup-inbox-close", undefined); await flushPromises();
      emit("popup-app-active", false);
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" }); await flushPromises();
      await tick(600);
      expect(activated()).toEqual(["b"]);
      expect(small()).toBe(false);
      expect(wrapper.find(".inbox-close-dialog").exists()).toBe(true);
    });

    it("still moves to the next primary request when a remote end finds nobody looking", async () => {
      await start("a", "b", "c");
      await later("b");
      emit("popup-app-active", false);
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" }); await flushPromises();
      await tick(600);
      expect(activated()).toEqual(["c"]);
      expect(small()).toBe(false);
    });

    it("jumps for a remote end while the app is active again", async () => {
      await start("a", "b");
      await later("b");
      emit("popup-app-active", false);
      emit("popup-app-active", true);
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" }); await flushPromises();
      await tick(600);
      expect(activated()).toEqual(["b"]);
      expect(small()).toBe(false);
    });

    it("only resizes the small window when another request ends while it is small", async () => {
      await start("a", "b", "c");
      await later("a"); await later("b"); await later("c");
      expect(small()).toBe(true);
      mock.invoke.mockClear();
      emit("popup-inbox-terminal", { requestId: "b", winner: "slack" }); await flushPromises();
      expect(layouts()).toHaveLength(1);
      expect(small()).toBe(true);
      expect(wrapper.findAll(".inbox-row")).toHaveLength(2);
      expect(calls("popup_inbox_activate")).toEqual([]);
    });

    it("keeps a small window small when the remembered request ends", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      mock.invoke.mockClear();
      emit("popup-inbox-terminal", { requestId: "b", winner: "slack" }); await flushPromises();
      await tick(600);
      expect(small()).toBe(true);
      expect(calls("popup_inbox_activate")).toEqual([]);
      expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
    });

    it("never touches the window when a request that is not open ends", async () => {
      await start("a", "b");
      emit("popup-inbox-terminal", { requestId: "b", winner: "slack" }); await flushPromises();
      expect(layouts().every(args => !args.collapsed)).toBe(true);
      expect(visible("a")).toBe(true);
    });
  });

  describe("round boundaries and refusal (R9)", () => {
    it("starts the next round expanded and unmarked after the last request ends in the small window", async () => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      await start("a");
      await later("a");
      expect(small()).toBe(true);
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" }); await flushPromises();
      await tick(600);
      expect(calls("popup_inbox_idle")).toHaveLength(1);
      // The window form is not carried over even before the next request arrives.
      mock.invoke.mockClear();
      emit("popup-inbox-reconcile", undefined); await flushPromises();
      expect(layouts()).toHaveLength(1);
      expect(layouts()[0].collapsed).toBeUndefined();
      mock.invoke.mockClear();
      emit("popup-inbox-show", request("z")); await flushPromises();
      expect(layouts().length).toBeGreaterThan(0);
      expect(layouts().every(args => !args.collapsed)).toBe(true);
      expect(visible("z")).toBe(true);
      expect(scope("z").secondary!.value).toBe(false);
      expect(scope("z").nativePreviewBlocked!.value).toBe(false);
      expect(bodyInert()).toBe(false);
    });

    it("keeps the window open without an alert when native refuses to shrink it", async () => {
      refuse = SHRINK_REFUSED;
      await start("a");
      await later("a");
      expect(wrapper.find(".inbox-error").exists()).toBe(false);
      expect(scope("a").secondary!.value).toBe(true);
      expect(bodyInert()).toBe(false);
      expect(scope("a").nativePreviewBlocked!.value).toBe(false);
      expect(visible("a")).toBe(true);
    });

    it("reports any other failure to shrink and keeps the window open", async () => {
      refuse = "layout unavailable";
      await start("a");
      await later("a");
      expect(wrapper.find(".inbox-error").text()).toBe("layout unavailable");
      expect(bodyInert()).toBe(false);
      expect(visible("a")).toBe(true);
    });

    it("returns to the small window when expanding fails", async () => {
      await start("a");
      await later("a");
      mock.invoke.mockImplementation(async (command: string) => { if (command === "popup_inbox_layout") throw "layout unavailable"; });
      await unlater("a");
      expect(wrapper.find(".inbox-error").text()).toBe("layout unavailable");
      expect(bodyInert()).toBe(true);
    });
  });
});
