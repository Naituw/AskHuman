import { defineComponent, h, onMounted, onBeforeUnmount } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInboxRequest } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";

// The small "pending" window as the page paints and drives it (docs/specs/popup-inbox-collapse.md C13–C17, C24, C27):
// window form transactions with their animation, the cross-fade state, the list's controls and the body swap.
// The window is native; here it is the `popup_inbox_*` calls.
const mock = vi.hoisted(() => ({
  invoke: vi.fn(), scopes: new Map<string, PopupScope>(),
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  mock.handlers.set(name, handler); return () => mock.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: vi.fn() }) }));
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
interface LayoutArgs { sidebar: boolean; collapsed?: { width: number; height: number } }
interface CommitArgs { revision: number; animation?: { durationMs: number; curve: number[] } }
interface Vm { look: string; markLater(id: string): void; unmarkLater(id: string): void }
const PREFERRED = 260;

describe("the small window and the body swap", () => {
  let wrapper: VueWrapper;
  let ids: string[];
  let revision: number;
  // Commits wait here while the test looks at the page in the middle of a window change.
  let hold: { promise: Promise<void>; release: () => void } | null;
  let reduced: boolean;
  let native: boolean;
  let seenLook: Array<[string, string]>;
  beforeEach(() => {
    revision = 0; hold = null; reduced = false; native = false; seenLook = [];
    mock.handlers.clear(); mock.invoke.mockReset(); mock.scopes.clear();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.stubGlobal("CSS", { escape: (s: string) => s });
    vi.stubGlobal("matchMedia", () => ({ get matches() { return reduced; }, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
    ids = ["a", "b"];
    mock.invoke.mockImplementation(async (command: string, args?: LayoutArgs & CommitArgs) => {
      seenLook.push([command, (wrapper?.vm as unknown as Vm | undefined)?.look ?? ""]);
      if (command === "popup_inbox_init") return { recovered: false, requests: ids.map(request) };
      if (command === "popup_inbox_layout") {
        // A lone request has no Sidebar when expanded, but native knows how wide its list would be.
        const list = args!.sidebar || !!args!.collapsed;
        return { revision: ++revision, sidebarWidth: list ? PREFERRED : 0, sidebarPreferred: PREFERRED, mainWidth: 560, mainHeight: 620,
          previewWidth: 0, limited: false, collapsed: !!args!.collapsed, ...(native ? { canvas: { left: 246, frozen: false } } : {}) };
      }
      if (command === "popup_inbox_commit" && hold) await hold.promise;
    });
  });
  afterEach(() => { hold?.release(); wrapper?.unmount(); vi.useRealTimers(); vi.unstubAllGlobals(); });
  async function start(...initial: string[]) {
    if (initial.length) ids = initial;
    wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
    mock.invoke.mockClear(); seenLook = [];
  }
  const vm = () => wrapper.vm as unknown as Vm;
  const calls = (command: string) => mock.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const layouts = () => calls("popup_inbox_layout") as LayoutArgs[];
  const commits = () => calls("popup_inbox_commit") as CommitArgs[];
  const root = () => wrapper.find(".inbox-root");
  const attr = (name: string) => root().attributes(name);
  const holdCommits = () => {
    let release!: () => void;
    const promise = new Promise<void>(resolve => { release = resolve; });
    hold = { promise, release };
    return () => { hold = null; release(); };
  };
  const tick = async (ms: number) => { await vi.advanceTimersByTimeAsync(ms); await flushPromises(); };
  // Putting a request away can jump to the next one, which swaps the body like a click on its row (C17: 126ms out, 174ms in).
  const settle = async () => { await flushPromises(); await tick(126 + 174); };
  const later = async (id: string) => { vm().markLater(id); await settle(); };
  const unlater = async (id: string) => { vm().unmarkLater(id); await settle(); };
  const visible = (id: string) => wrapper.find(`[data-inbox-request="${id}"]`).isVisible();

  describe("the form change and its animation", () => {
    it("shrinks with the collapse animation to a window as wide as the list", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      const size = layouts()[layouts().length - 1];
      expect(size.collapsed).toEqual({ width: PREFERRED, height: expect.any(Number) });
      expect(size.collapsed!.height).toBeGreaterThanOrEqual(1);
      expect(commits()[commits().length - 1]).toEqual({ revision: revision, animation: { durationMs: 440, curve: [0.2, 0, 0, 1] } });
    });

    it("expands with the longer animation and resizes with the short one", async () => {
      await start("a", "b", "c");
      await later("a"); await later("b"); await later("c");
      mock.invoke.mockClear();
      mock.handlers.get("popup-inbox-terminal")!({ payload: { requestId: "b", winner: "slack" } }); await flushPromises();
      expect(commits().map(args => args.animation?.durationMs)).toEqual([240]);
      mock.invoke.mockClear();
      await unlater("c");
      expect(commits().map(args => args.animation?.durationMs)).toEqual([480]);
    });

    it("changes the window without an animation when motion is reduced", async () => {
      reduced = true;
      await start("a", "b");
      await later("a"); await later("b");
      await unlater("a");
      expect(commits().length).toBeGreaterThanOrEqual(2);
      expect(commits().every(args => !("animation" in args))).toBe(true);
    });

    it("lays a single request's list out at the width native prefers, and keeps the list as the small window", async () => {
      await start("a");
      expect(wrapper.find(".inbox-sidebar").exists()).toBe(false);
      await later("a");
      expect(layouts()).toHaveLength(1);
      expect(layouts()[0].sidebar).toBe(true);
      expect(layouts()[0].collapsed!.width).toBe(PREFERRED);
      expect(wrapper.find(".inbox-error").exists()).toBe(false);
      expect(wrapper.find(".inbox-sidebar").exists()).toBe(true);
      expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
    });

    it("paints the new form only when native starts the change, not ahead of it", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      const looks = seenLook.filter(([command]) => command === "popup_inbox_layout" || command === "popup_inbox_commit");
      // The layout of the second marking is prepared while the window still looks expanded; its commit follows the flip.
      expect(looks.slice(-2)).toEqual([["popup_inbox_layout", "expanded"], ["popup_inbox_commit", "collapsed"]]);
    });

    it("states the window form to the page while it changes", async () => {
      await start("a", "b");
      expect([attr("data-view"), attr("data-primary"), attr("data-sidebar"), attr("data-settled"), attr("data-shifting")])
        .toEqual(["expanded", "yes", "on", "no", "no"]);
      await later("a");
      const release = holdCommits();
      void later("b");
      await flushPromises();
      expect([attr("data-view"), attr("data-primary"), attr("data-settled"), attr("data-shifting")]).toEqual(["collapsed", "no", "no", "yes"]);
      release(); await flushPromises();
      expect([attr("data-view"), attr("data-settled"), attr("data-shifting")]).toEqual(["collapsed", "yes", "no"]);
      await unlater("b");
      expect([attr("data-view"), attr("data-primary"), attr("data-settled")]).toEqual(["expanded", "yes", "no"]);
    });

    it("times the cross-fade with the window change and with nothing else", async () => {
      await start("a", "b");
      const duration = () => root().attributes("style")!.match(/--ms: (\d+)ms/)![1];
      expect(duration()).toBe("0");
      await later("a");
      const release = holdCommits();
      void later("b");
      await flushPromises();
      expect(duration()).toBe("440");
      release(); await flushPromises();
      // A Sidebar that appears or a window that is resized later must not fade.
      expect(duration()).toBe("0");
    });

    it("keeps a single request's list in place until the window has grown, then removes it", async () => {
      await start("a");
      await later("a");
      expect(wrapper.find(".inbox-sidebar").exists()).toBe(true);
      const release = holdCommits();
      void unlater("a");
      await flushPromises();
      expect(attr("data-view")).toBe("expanded");
      expect(attr("data-sidebar")).toBe("off");
      expect(wrapper.find(".inbox-sidebar").exists()).toBe(true);
      release(); await flushPromises();
      expect(wrapper.find(".inbox-sidebar").exists()).toBe(false);
      expect(visible("a")).toBe(true);
    });

    it("keeps the small window and its list when expanding fails", async () => {
      await start("a");
      await later("a");
      const failing = mock.invoke.getMockImplementation()!;
      // The lone request's expanded layout has no list; the window then fails to change.
      mock.invoke.mockImplementation(async (command: string, args?: LayoutArgs) => {
        if (command === "popup_inbox_commit") throw "window unavailable";
        return failing(command, args);
      });
      await unlater("a");
      expect(wrapper.find(".inbox-error").text()).toBe("window unavailable");
      expect(attr("data-view")).toBe("collapsed");
      expect(vm().look).toBe("collapsed");
      expect(attr("data-settled")).toBe("yes");
      expect(wrapper.find(".inbox-sidebar").exists()).toBe(true);
      expect(root().attributes("style")).toContain(`--inbox-sidebar: ${PREFERRED}px`);
    });

    it("goes back to the expanded look when the window fails after the change was painted", async () => {
      await start("a", "b");
      await later("a");
      const normal = mock.invoke.getMockImplementation()!;
      mock.invoke.mockImplementation(async (command: string, args?: LayoutArgs) => {
        if (command === "popup_inbox_commit") throw "window unavailable";
        return normal(command, args);
      });
      await later("b");
      expect(wrapper.find(".inbox-error").text()).toBe("window unavailable");
      expect(attr("data-view")).toBe("expanded");
      expect(vm().look).toBe("expanded");
      expect(attr("data-settled")).toBe("no");
      expect(wrapper.find(".inbox-body").attributes("inert")).toBe("false");
    });

    it("hides the traffic lights only in a native window", async () => {
      await start("a", "b");
      expect(attr("data-lights")).toBe("keep");
      wrapper.unmount();
      native = true;
      await start("a", "b");
      expect(attr("data-lights")).toBe("hide");
    });

    it("keeps the expanded look when native refuses to shrink", async () => {
      await start("a", "b");
      mock.invoke.mockImplementation(async (command: string, args?: LayoutArgs) => {
        if (command === "popup_inbox_layout") {
          if (args?.collapsed) throw "The window cannot shrink while it is maximized or full screen";
          return { revision: ++revision, sidebarWidth: PREFERRED, sidebarPreferred: PREFERRED, mainWidth: 560, mainHeight: 620, previewWidth: 0, limited: false };
        }
      });
      await later("a"); await later("b");
      expect(wrapper.find(".inbox-error").exists()).toBe(false);
      expect(attr("data-view")).toBe("expanded");
      expect(vm().look).toBe("expanded");
      expect(wrapper.find(".inbox-body").attributes("inert")).toBe("false");
    });

    it("starts the next round with an expanded look", async () => {
      await start("a");
      await later("a");
      expect(attr("data-view")).toBe("collapsed");
      mock.handlers.get("popup-inbox-terminal")!({ payload: { requestId: "a", winner: "slack" } });
      await tick(600);
      expect(attr("data-view")).toBe("expanded");
      expect(attr("data-settled")).toBe("no");
    });
  });

  describe("the list's controls", () => {
    async function allMarked() {
      await start("a", "b");
      await later("a"); await later("b");
      expect(attr("data-view")).toBe("collapsed");
      mock.invoke.mockClear();
    }

    it("shows the count in the heading and marks every row as put away", async () => {
      await allMarked();
      expect(wrapper.find(".inbox-heading").text()).toContain("Waiting for answers");
      expect(wrapper.find(".inbox-count").text()).toBe("2");
      expect(wrapper.findAll(".inbox-row").every(row => row.classes().includes("secondary"))).toBe(true);
      // Nothing is open in the small window.
      expect(wrapper.findAll(".inbox-row").some(row => row.classes().includes("selected"))).toBe(false);
    });

    it("opens the request the user was last on from the heading's expand icon", async () => {
      await allMarked();
      await wrapper.find(".mini-expand").trigger("click"); await flushPromises();
      expect(layouts().every(args => !args.collapsed)).toBe(true);
      expect(attr("data-view")).toBe("expanded");
      expect(visible("b")).toBe(true);
    });

    it("goes back to the small window from the heading's shrink icon while only put-away requests remain", async () => {
      await allMarked();
      await wrapper.findAll(".inbox-row")[0].trigger("click"); await flushPromises();
      expect(attr("data-view")).toBe("expanded");
      expect(attr("data-primary")).toBe("no");
      mock.invoke.mockClear();
      await wrapper.find(".mini-shrink").trigger("click"); await flushPromises();
      expect(attr("data-view")).toBe("collapsed");
      expect(layouts()[0].collapsed).toBeTruthy();
    });

    it("leaves the window alone when the shrink icon is used while a request is waiting", async () => {
      await start("a", "b");
      await later("b");
      mock.invoke.mockClear();
      await wrapper.find(".mini-shrink").trigger("click"); await flushPromises();
      expect(layouts()).toEqual([]);
      expect(attr("data-view")).toBe("expanded");
    });

    it("marks and restores from the row's flag without opening the row", async () => {
      await start("a", "b", "c");
      const flag = (index: number) => wrapper.findAll(".row-flag")[index];
      await flag(2).trigger("click"); await flushPromises();
      expect(wrapper.findAll(".inbox-row")[2].classes()).toContain("secondary");
      expect(flag(2).classes()).toContain("is-secondary");
      expect(calls("popup_inbox_activate")).toEqual([]);
      expect(layouts()).toEqual([]);
      await flag(2).trigger("click"); await flushPromises();
      expect(wrapper.findAll(".inbox-row")[2].classes()).not.toContain("secondary");
      expect(calls("popup_inbox_activate")).toEqual([]);
    });
  });

  describe("switching between requests (C17)", () => {
    const row = (index: number) => wrapper.findAll(".inbox-row")[index];
    const phase = (id: string) => wrapper.find(`[data-inbox-request="${id}"]`).attributes("data-answer-phase");

    it("fades the body out, then fades the next one in", async () => {
      await start("a", "b");
      await row(1).trigger("click"); await flushPromises();
      // The row follows the click at once; the body being left fades out.
      expect(row(1).classes()).toContain("selected");
      expect(phase("a")).toBe("out");
      expect(visible("a")).toBe(true);
      expect(wrapper.find(".inbox-body").attributes("inert")).toBe("true");
      await tick(126);
      expect(visible("a")).toBe(false);
      expect(visible("b")).toBe(true);
      expect(phase("b")).toBe("in");
      expect(wrapper.find(".inbox-body").attributes("inert")).toBe("false");
      await tick(174);
      expect(phase("b")).toBeUndefined();
    });

    it("hides the old request's native preview while its body fades out and lets the next one place its own", async () => {
      await start("a", "b");
      const hidden = (id: string) => mock.scopes.get(id)!.nativePreviewBlocked!.value;
      expect(hidden("a")).toBe(false);
      const release = holdCommits();
      await row(1).trigger("click"); await flushPromises();
      expect([hidden("a"), hidden("b")]).toEqual([true, true]);
      await tick(126);
      // The window is still changing, but the incoming request's preview is placed before its body is revealed.
      expect([hidden("a"), hidden("b")]).toEqual([true, false]);
      release(); await flushPromises();
      expect([hidden("a"), hidden("b")]).toEqual([false, false]);
    });

    it("keeps the next body hidden until the window has its final geometry", async () => {
      await start("a", "b");
      const release = holdCommits();
      await row(1).trigger("click"); await tick(126);
      expect(visible("b")).toBe(true);
      expect(phase("b")).toBe("prepare");
      release(); await flushPromises();
      expect(phase("b")).toBe("in");
    });

    it("does not queue clicks while the old body fades out", async () => {
      await start("a", "b", "c");
      await row(1).trigger("click"); await flushPromises();
      await row(2).trigger("click"); await flushPromises();
      await tick(126); await tick(174);
      expect(calls("popup_inbox_activate").map(args => args.requestId)).toEqual(["b"]);
      expect(visible("b")).toBe(true);
    });

    it("replaces a body that is still fading in without fading it out again", async () => {
      await start("a", "b", "c");
      await row(1).trigger("click"); await tick(126);
      expect(phase("b")).toBe("in");
      await row(2).trigger("click"); await flushPromises();
      expect(visible("c")).toBe(true);
      expect(visible("b")).toBe(false);
      expect(phase("c")).toBe("in");
    });

    it("switches at once with reduced motion", async () => {
      reduced = true;
      await start("a", "b");
      await row(1).trigger("click"); await flushPromises();
      expect(visible("b")).toBe(true);
      expect(visible("a")).toBe(false);
      expect(phase("b")).toBeUndefined();
    });

    it("keeps the body that was fading out when the next request ends meanwhile", async () => {
      await start("a", "b");
      await row(1).trigger("click"); await flushPromises();
      mock.handlers.get("popup-inbox-terminal")!({ payload: { requestId: "b", winner: "slack" } });
      await tick(126);
      expect(visible("a")).toBe(true);
      expect(phase("a")).toBeUndefined();
      await tick(600);
      expect(wrapper.findAll(".inbox-row")).toHaveLength(1);
      expect(visible("a")).toBe(true);
    });

    it("swaps the body when putting the open request away jumps to the next one", async () => {
      await start("a", "b");
      vm().markLater("a"); await flushPromises();
      // The mark is made at once, the row of the next request is selected, and the body being left fades out.
      expect(row(0).classes()).toContain("secondary");
      expect(row(1).classes()).toContain("selected");
      expect(phase("a")).toBe("out");
      expect(visible("a")).toBe(true);
      expect(wrapper.find(".inbox-body").attributes("inert")).toBe("true");
      await tick(126);
      expect(visible("a")).toBe(false);
      expect(visible("b")).toBe(true);
      expect(phase("b")).toBe("in");
      await tick(174);
      expect(phase("b")).toBeUndefined();
    });

    it("ignores a second put-away while the body fades out, since it would belong to a request not seen yet", async () => {
      await start("a", "b", "c");
      vm().markLater("a"); await flushPromises();
      vm().markLater("b"); await flushPromises();
      await tick(126 + 174);
      expect(row(1).classes()).not.toContain("secondary");
      expect(visible("b")).toBe(true);
      expect(attr("data-view")).toBe("expanded");
    });

    it("jumps at once with reduced motion when putting the open request away", async () => {
      reduced = true;
      await start("a", "b");
      vm().markLater("a"); await flushPromises();
      expect(visible("b")).toBe(true);
      expect(visible("a")).toBe(false);
      expect(phase("b")).toBeUndefined();
    });

    it("keeps the body that was fading out when the request it would jump to ends meanwhile", async () => {
      await start("a", "b");
      vm().markLater("a"); await flushPromises();
      mock.handlers.get("popup-inbox-terminal")!({ payload: { requestId: "b", winner: "slack" } });
      await tick(126);
      expect(visible("a")).toBe(true);
      expect(phase("a")).toBeUndefined();
    });

    it("does not swap when the window opens from the small one", async () => {
      await start("a", "b");
      await later("a"); await later("b");
      await row(0).trigger("click"); await flushPromises();
      expect(visible("a")).toBe(true);
      expect(phase("a")).toBeUndefined();
    });

    it("hands over to the answer transition when a request is answered during the fade", async () => {
      await start("a", "b");
      await row(1).trigger("click"); await tick(126);
      mock.handlers.get("popup-inbox-terminal")!({ payload: { requestId: "b", winner: "popup", completion: "sent" } });
      await flushPromises();
      expect(wrapper.attributes("data-completion-phase")).toBe("out");
      expect(phase("b")).toBe("out");
    });
  });
});
