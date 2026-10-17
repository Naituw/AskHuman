import { defineComponent, h, onBeforeUnmount, onMounted } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInboxRequest } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";

// Files dragged over the inbox window (docs/specs/popup-inbox-collapse.md C28-C32; R13, R15, R16): resting over the small
// window opens it, resting on a Sidebar row opens that request, the edges scroll, and a drop goes to what is under the
// pointer. The native drag events arrive as `onDragDropEvent` payloads; the window itself is native, so what is where on
// the page comes from rectangles registered per element (jsdom lays nothing out).
interface Rect { left: number; top: number; right: number; bottom: number; width: number; height: number }
const mock = vi.hoisted(() => ({
  invoke: vi.fn(), scopes: new Map<string, PopupScope>(),
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  drag: null as null | ((event: { payload: unknown }) => void),
  focus: vi.fn(),
  rects: new Map<Element, Rect | (() => Rect)>(),
  // What each fake request body was handed, which question each one is on, and how each one is set up.
  received: [] as { id: string; paths: string[]; question: number }[],
  current: new Map<string, number>(),
  stacked: new Map<string, number>(),
  previews: new Set<string>(),
  draggingOut: new Set<string>(),
  neverReady: new Set<string>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  mock.handlers.set(name, handler); return () => mock.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: mock.focus }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async (handler: (event: { payload: unknown }) => void) => {
  mock.drag = handler; return () => { mock.drag = null; };
} }) }));
vi.mock("../PopupView.vue", () => ({ default: defineComponent({ props: ["scope"], setup(props) {
  const scope = props.scope as PopupScope;
  const id = scope.requestId;
  mock.scopes.set(id, scope);
  mock.current.set(id, 0);
  scope.dropReceiver?.({
    currentQuestion: () => mock.current.get(id) ?? 0,
    draggingOut: () => mock.draggingOut.has(id),
    endDragOut: () => { mock.draggingOut.delete(id); },
    acceptDrop: async (paths, question) => {
      mock.received.push({ id, paths, question });
      // Dropping on a card of the stacked layout is working on that question.
      if (mock.stacked.has(id)) mock.current.set(id, question);
      return paths.length;
    },
  });
  onMounted(() => { if (!mock.neverReady.has(id)) scope.ready(id); });
  onBeforeUnmount(() => { scope.dropReceiver?.(null); mock.scopes.delete(id); });
  return () => {
    const aim = scope.dropAim?.value;
    return h("div", { class: "fake-request", "data-aim": !aim ? "none" : aim.question === null ? "request" : String(aim.question) }, [
      h("div", { class: "content" }, mock.stacked.has(id) ? Array.from({ length: mock.stacked.get(id)! }, (_, i) => h("div", { class: "q-card", "data-q-index": i })) : h("div", { class: "answer" })),
      h("div", { class: "footer" }, "Send"),
      ...(mock.previews.has(id) ? [h("div", { class: "attachment-preview" })] : []),
    ]);
  };
} }) }));
import PopupInboxView from "../PopupInboxView.vue";

function ask(id: string, questions = 1): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project: "/project", source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] },
      questions: Array.from({ length: questions }, (_, i) => ({ message: i ? `Question ${i + 1} of ${id}` : `Request ${id}`, predefinedOptions: [] })) } } };
}
function confirmation(id: string): PopupInboxRequest {
  return { ...ask(id), kind: "permission", interaction: { type: "confirm", request: { id, title: `Request ${id}`, context: [],
    detail: { kind: "none" }, choices: [], presentation: "inline", dismissActionId: "deny", createdAtMs: 1, expiresAtMs: 2 } } as unknown as PopupInboxRequest["interaction"] };
}
function emit(name: string, payload: unknown) { mock.handlers.get(name)!({ payload }); }
interface LayoutArgs { sidebar: boolean; collapsed?: { width: number; height: number } }

// The page the way native and CSS would lay it out. Rows are 40 tall; in the small window the list starts right under the
// heading, in the open window one row lower. With a canvas (macOS) the page starts at x = 360 and the small window
// leaves 441 of the canvas sticking out below it.
const ROW = 40, ROW_COLLAPSED = 60, ROW_EXPANDED = 100, LEFT = 360, EXCESS = 441, BODY = 246;
const CONTENT = { top: 40, height: 500 };

describe("files dragged over the window", () => {
  let wrapper: VueWrapper;
  let requests: PopupInboxRequest[];
  let revision: number;
  let canvas: boolean;
  // What native last laid out: the small window or the open one.
  let collapsed: boolean;
  beforeEach(() => {
    requests = ["a", "b", "c"].map(id => ask(id)); revision = 0; canvas = false; collapsed = false;
    mock.handlers.clear(); mock.invoke.mockReset(); mock.scopes.clear(); mock.focus.mockReset(); mock.drag = null;
    mock.rects.clear(); mock.received.length = 0; mock.current.clear(); mock.stacked.clear(); mock.previews.clear(); mock.draggingOut.clear(); mock.neverReady.clear();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "performance"] });
    vi.stubGlobal("CSS", { escape: (s: string) => s });
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
      const rect = mock.rects.get(this);
      return (typeof rect === "function" ? rect() : rect ?? { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }) as DOMRect;
    });
    // The smallest registered rectangle that holds the point: what the browser would report as the topmost element there.
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: (x: number, y: number) => {
      // Native lays the window out as the page changes; so does this, whenever a point is looked up.
      arrange();
      let best: Element | null = null, area = Infinity;
      for (const [element, entry] of mock.rects) {
        if (!element.isConnected) continue;
        const r = typeof entry === "function" ? entry() : entry;
        if (x >= r.left && x < r.right && y >= r.top && y < r.bottom && r.width * r.height <= area) { best = element; area = r.width * r.height; }
      }
      return best;
    } });
    mock.invoke.mockImplementation(async (command: string, args?: LayoutArgs & { revision?: number }) => {
      if (command === "popup_inbox_init") return { recovered: false, requests };
      if (command === "popup_inbox_layout") {
        collapsed = !!args?.collapsed;
        const base = { revision: ++revision, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0, limited: false, collapsed };
        return canvas ? { ...base, canvas: { left: 606, width: 1166, height: 620, frozen: collapsed }, frame: collapsed ? { width: 240, height: 179 } : { width: 806, height: 620 } } : base;
      }
      if (command === "popup_inbox_finish" && canvas) {
        return collapsed ? undefined : { revision, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0, limited: false, collapsed: false,
          canvas: { left: 606, width: 1166, height: 620, frozen: false }, frame: { width: 806, height: 620 } };
      }
    });
  });
  afterEach(() => {
    wrapper?.unmount(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks();
    Reflect.deleteProperty(document, "elementFromPoint");
  });
  const tick = async (ms: number) => { await vi.advanceTimersByTimeAsync(ms); await flushPromises(); };
  const calls = (command: string) => mock.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const layouts = () => calls("popup_inbox_layout") as LayoutArgs[];
  const small = () => collapsed;
  const root = () => wrapper.find(".inbox-root");
  const view = () => root().attributes("data-view");
  const row = (index: number) => wrapper.findAll(".inbox-row")[index];
  const visible = (id: string) => { const body = wrapper.find(`[data-inbox-request="${id}"]`); return body.exists() && body.isVisible(); };
  const aimOf = (id: string) => wrapper.find(`[data-inbox-request="${id}"] .fake-request`).attributes("data-aim");
  const dragging = () => root().attributes("data-dragging") === "yes";
  const vm = () => wrapper.vm as unknown as { markLater(id: string): void };
  const later = async (id: string) => { vm().markLater(id); await flushPromises(); await tick(126 + 174); };

  function place(target: Element | undefined, left: number, top: number, width: number, height: number) {
    if (target) mock.rects.set(target, { left, top, right: left + width, bottom: top + height, width, height });
  }
  /** Lay the page out for what it shows now (a single request has no Sidebar). It runs whenever a point is looked up. */
  function arrange() {
    mock.rects.clear();
    const find = (selector: string) => { const found = wrapper.find(selector); return found.exists() ? found.element : undefined; };
    const collapsed = view() === "collapsed";
    const x = canvas ? LEFT : 0, first = collapsed ? ROW_COLLAPSED : ROW_EXPANDED;
    place(find(".inbox-sidebar"), x, 0, 240, collapsed ? 179 : 620);
    place(find(".inbox-heading"), x, 0, 240, first - 30);
    const navigation = find(".inbox-navigation");
    place(navigation, x, first - 30, 240, (collapsed ? 179 : 620) - first + 30);
    // The list scrolls: its rows move with it.
    wrapper.findAll(".inbox-entry").forEach((entry, i) => {
      mock.rects.set(entry.element, () => {
        const top = first + ROW * i - (navigation?.scrollTop ?? 0);
        return { left: x, top, right: x + 240, bottom: top + ROW, width: 240, height: ROW };
      });
    });
    if (collapsed) return;
    place(find(".inbox-body"), x + BODY, 0, 560, 620);
    const open = wrapper.findAll("[data-inbox-request]").find(body => body.isVisible());
    if (!open) return;
    const content = open.find(".content").element;
    place(content, x + BODY, CONTENT.top, 560, CONTENT.height);
    open.findAll(".q-card").forEach((card, i) => {
      mock.rects.set(card.element, () => {
        const top = CONTENT.top + 20 + 150 * i - content.scrollTop;
        return { left: x + BODY + 10, top, right: x + BODY + 550, bottom: top + 130, width: 540, height: 130 };
      });
    });
    place(open.find(".footer").element, x + BODY, 540, 560, 80);
    const preview = open.find(".attachment-preview");
    if (preview.exists()) place(preview.element, x + BODY + 300, CONTENT.top, 260, CONTENT.height);
  }
  /** Give an element the scroll metrics jsdom does not have. */
  function scroller(element: Element, height: number, client: number) {
    let top = 0;
    Object.defineProperties(element, {
      scrollTop: { configurable: true, get: () => top, set: (value: number) => { top = Math.max(0, Math.min(height - client, value)); } },
      scrollHeight: { configurable: true, get: () => height },
      clientHeight: { configurable: true, get: () => client },
    });
  }

  /** Native positions are the page's, unless the canvas is fixed: then x is from the window's edge and y from the canvas' top. */
  function native(x: number, y: number) {
    if (!canvas) return { x, y };
    return { x: x - LEFT, y: y + (small() ? EXCESS : 0) };
  }
  async function send(type: "enter" | "over" | "drop" | "leave", x = 0, y = 0, paths?: string[]) {
    mock.drag!({ payload: { type, ...(paths ? { paths } : {}), position: native(x, y) } });
    await flushPromises();
  }
  const rowY = (index: number) => (view() === "collapsed" ? ROW_COLLAPSED : ROW_EXPANDED) + ROW * index + ROW / 2;
  const onRow = (index: number, type: "enter" | "over" = "over") => send(type, (canvas ? LEFT : 0) + 120, rowY(index));
  const onBody = (type: "enter" | "over" = "over", y = CONTENT.top + 250) => send(type, (canvas ? LEFT : 0) + BODY + 280, y);
  const onHeading = (type: "enter" | "over" = "over") => send(type, (canvas ? LEFT : 0) + 120, 20);

  async function start(...ids: string[]) {
    if (ids.length) requests = ids.map(id => ask(id));
    requests = requests.map(r => r.interaction.type === "ask" && mock.stacked.has(r.requestId) ? ask(r.requestId, mock.stacked.get(r.requestId)) : r);
    wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
  }
  /** Everything put away: the small window. */
  async function smallWindow(...ids: string[]) {
    await start(...ids);
    for (const id of requests.map(r => r.requestId)) await later(id);
    expect(small()).toBe(true);
    arrange();
  }
  /** Every request put away, then one opened from the small window: an expanded window with nothing waiting (a peek). */
  async function peek(open = 0) {
    await smallWindow();
    await row(open).trigger("click"); await flushPromises();
    expect(view()).toBe("expanded");
    arrange();
    mock.focus.mockClear(); mock.invoke.mockClear();
  }

  describe("over the small window", () => {
    it("opens it onto the row under the pointer after resting there, without taking the focus", async () => {
      await smallWindow();
      await onRow(1, "enter");
      expect(dragging()).toBe(true);
      expect(row(1).classes()).toContain("dwelling");
      expect(row(1).attributes("style")).toContain("--dwell-ms: 500ms");
      await tick(499);
      expect(small()).toBe(true);
      await tick(1);
      expect(small()).toBe(false);
      expect(view()).toBe("expanded");
      expect(visible("b")).toBe(true);
      expect(mock.focus).not.toHaveBeenCalled();
    });

    it("opens the request it was last on when the pointer rests on the heading instead of a row", async () => {
      await smallWindow();
      await onHeading("enter");
      // The wait is shown on the row it will open.
      expect(row(2).classes()).toContain("dwelling");
      await tick(500);
      expect(view()).toBe("expanded");
      expect(visible("c")).toBe(true);
    });

    it("opens nothing when the pointer leaves before the wait is over", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(400);
      await send("leave");
      expect(dragging()).toBe(false);
      expect(row(1).classes()).not.toContain("dwelling");
      mock.invoke.mockClear();
      await tick(3000);
      expect(layouts()).toEqual([]);
      expect(small()).toBe(true);
    });

    it("starts the wait again when the pointer moves to another row", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(300);
      await onRow(2);
      expect(row(1).classes()).not.toContain("dwelling");
      expect(row(2).classes()).toContain("dwelling");
      await tick(300);
      expect(small()).toBe(true);
      await tick(200);
      expect(visible("c")).toBe(true);
    });

    it("keeps waiting while the pointer moves within the same row", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(300);
      await send("over", 130, rowY(1) - 5);
      await send("over", 140, rowY(1) + 5);
      await tick(200);
      expect(visible("b")).toBe(true);
    });

    it("lights the rows that can take files and, among them, the one under the pointer", async () => {
      requests = [ask("a"), confirmation("b"), ask("c")];
      await smallWindow();
      await onRow(0, "enter");
      expect(row(0).classes()).toEqual(expect.arrayContaining(["can-drop", "drop-target"]));
      expect(row(1).classes()).not.toContain("can-drop");
      expect(row(2).classes()).toContain("can-drop");
      expect(row(2).classes()).not.toContain("drop-target");
      await onRow(1);
      // A request that cannot take files is not a target, though resting on it still opens it.
      expect(row(1).classes()).not.toContain("drop-target");
      expect(row(0).classes()).not.toContain("drop-target");
    });

    it("forgets the lights when the drag is over", async () => {
      await smallWindow();
      await onRow(0, "enter");
      await send("leave");
      expect(row(0).classes()).not.toContain("drop-target");
      expect(dragging()).toBe(false);
    });
  });

  describe("dropping on the small window", () => {
    it("hands the files to the row's request at once, shows it, and brings the window forward", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await send("drop", 120, rowY(1), ["/tmp/x.txt", "/tmp/y.txt"]);
      expect(mock.received).toEqual([{ id: "b", paths: ["/tmp/x.txt", "/tmp/y.txt"], question: 0 }]);
      expect(view()).toBe("expanded");
      expect(visible("b")).toBe(true);
      expect(mock.focus).toHaveBeenCalledTimes(1);
      expect(dragging()).toBe(false);
      expect(wrapper.find(".inbox-notice").text()).toBe("Added 2 files to “Request b”");
    });

    it("needs no rest: a drop on the heading goes to the request the window would open", async () => {
      await smallWindow();
      await send("drop", 120, 20, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "c", paths: ["/tmp/x.txt"], question: 0 }]);
      expect(visible("c")).toBe(true);
      expect(wrapper.find(".inbox-notice").text()).toBe("Added 1 file to “Request c”");
    });

    it("ignores a drop on a request that cannot take files, and leaves the window as it was", async () => {
      requests = [ask("a"), confirmation("b"), ask("c")];
      await smallWindow();
      mock.invoke.mockClear();
      await send("drop", 120, rowY(1), ["/tmp/x.txt"]);
      expect(mock.received).toEqual([]);
      expect(small()).toBe(true);
      expect(layouts()).toEqual([]);
      expect(mock.focus).not.toHaveBeenCalled();
      expect(wrapper.find(".inbox-notice").exists()).toBe(false);
    });

    it("hands a drop released just outside everything to the row it was aimed at a moment ago", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await send("drop", 120, 900, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "b", paths: ["/tmp/x.txt"], question: 0 }]);
    });

    it("ignores a drop that carries no files", async () => {
      await smallWindow();
      await send("drop", 120, rowY(1), []);
      expect(mock.received).toEqual([]);
      expect(small()).toBe(true);
    });

    it("forgets the notice after a few seconds", async () => {
      await smallWindow();
      await send("drop", 120, rowY(0), ["/tmp/x.txt"]);
      expect(wrapper.find(".inbox-notice").exists()).toBe(true);
      await tick(2600);
      expect(wrapper.find(".inbox-notice").exists()).toBe(false);
    });

    it("opens a request that was never opened to put the files in it", async () => {
      await start("a", "b", "c");
      // Put away without ever being visited: only the open one is mounted.
      vm().markLater("c"); vm().markLater("b"); await flushPromises();
      await later("a");
      expect(small()).toBe(true);
      expect(wrapper.find('[data-inbox-request="c"]').exists()).toBe(false);
      arrange();
      await send("drop", 120, rowY(2), ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "c", paths: ["/tmp/x.txt"], question: 0 }]);
      expect(visible("c")).toBe(true);
    });

    it("says so when a request that was never opened does not get ready in time, and puts nothing anywhere", async () => {
      mock.neverReady.add("c");
      await start("a", "b", "c");
      vm().markLater("c"); vm().markLater("b"); await flushPromises();
      await later("a");
      arrange();
      await send("drop", 120, rowY(2), ["/tmp/x.txt"]);
      expect(wrapper.find(".inbox-error").exists()).toBe(false);
      await tick(3000);
      expect(wrapper.find(".inbox-error").text()).toBe("Could not add the files to “Request c”");
      expect(mock.received).toEqual([]);
      expect(small()).toBe(true);
    });
  });

  describe("giving up", () => {
    it("shrinks the window back after a drag that opened it leaves for good", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(500);
      expect(small()).toBe(false);
      arrange();
      await send("leave");
      await tick(599);
      expect(small()).toBe(false);
      await tick(1);
      expect(small()).toBe(true);
      expect(view()).toBe("collapsed");
    });

    it("keeps the window when the drag is back within that time, and starts over when it leaves again", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(500);
      arrange();
      await send("leave");
      await tick(400);
      await onBody("enter");
      await tick(2000);
      expect(small()).toBe(false);
      await send("leave");
      await tick(600);
      expect(small()).toBe(true);
    });

    it("does not shrink a window the user opened themselves", async () => {
      await peek();
      await onBody("enter");
      await send("leave");
      mock.invoke.mockClear();
      await tick(3000);
      expect(layouts()).toEqual([]);
      expect(view()).toBe("expanded");
    });

    it("does not shrink the window after a drop: it is what the user was working with", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(500);
      arrange();
      await send("drop", (canvas ? LEFT : 0) + BODY + 280, CONTENT.top + 250, ["/tmp/x.txt"]);
      await send("leave");
      mock.invoke.mockClear();
      await tick(3000);
      expect(layouts()).toEqual([]);
      expect(view()).toBe("expanded");
    });

    it("keeps a window that something else holds open: a request arrived while the drag was in it", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(500);
      expect(view()).toBe("expanded");
      arrange();
      emit("popup-inbox-show", ask("z")); await flushPromises();
      await send("leave");
      mock.invoke.mockClear();
      await tick(3000);
      expect(layouts()).toEqual([]);
      expect(view()).toBe("expanded");
      expect(wrapper.find(".inbox-root").attributes("data-primary")).toBe("yes");
    });
  });

  describe("over the open window", () => {
    it("switches to another request's body after resting on its row", async () => {
      await peek(0);
      await onRow(1, "enter");
      expect(row(1).classes()).toContain("dwelling");
      expect(row(1).attributes("style")).toContain("--dwell-ms: 250ms");
      await tick(249);
      expect(visible("a")).toBe(true);
      await tick(1 + 126 + 174);
      expect(visible("b")).toBe(true);
      expect(visible("a")).toBe(false);
      expect(mock.focus).not.toHaveBeenCalled();
    });

    it("waits for nothing on the open request's own row, or anywhere in its body", async () => {
      await peek(0);
      await onRow(0, "enter");
      expect(wrapper.findAll(".inbox-row.dwelling")).toHaveLength(0);
      await onBody();
      expect(wrapper.findAll(".inbox-row.dwelling")).toHaveLength(0);
      mock.invoke.mockClear();
      await tick(2000);
      expect(layouts()).toEqual([]);
      expect(visible("a")).toBe(true);
    });

    it("does not switch for a row the pointer only passes over", async () => {
      await peek(0);
      await onRow(1, "enter");
      await tick(200);
      await onRow(2);
      await tick(200);
      await onBody();
      await tick(1000);
      expect(visible("a")).toBe(true);
    });

    it("does not start a new wait under a pointer that did not move while the window changed under it", async () => {
      await smallWindow();
      // In the small window the pointer rests on row 1 (b); once open, the list sits one row lower, so row 0 (a) is under it.
      await onRow(1, "enter");
      await tick(500);
      expect(visible("b")).toBe(true);
      arrange();
      await send("over", 120, ROW_COLLAPSED + ROW * 1 + ROW / 2);
      await tick(1000);
      // Still on b: the pointer has not moved since the window opened.
      expect(visible("b")).toBe(true);
      // It moves, and the row it rests on starts a wait.
      await send("over", 120, ROW_COLLAPSED + ROW * 1 + ROW / 2 + 10);
      await tick(250 + 126 + 174);
      expect(visible("a")).toBe(true);
    });

    it("takes no files over an attachment preview, and does not wait for anything there", async () => {
      mock.previews.add("a");
      await peek(0);
      await send("enter", BODY + 400, 300);
      expect(aimOf("a")).toBe("none");
      expect(wrapper.findAll(".inbox-row.dwelling")).toHaveLength(0);
      await send("drop", BODY + 400, 300, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([]);
      expect(mock.focus).not.toHaveBeenCalled();
      // Beside it, the answer area takes them.
      await send("drop", BODY + 100, 300, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "a", paths: ["/tmp/x.txt"], question: 0 }]);
    });

    it("aims again once the window has opened under a pointer that has not moved", async () => {
      await smallWindow();
      await onRow(1, "enter");
      expect(row(1).classes()).toContain("drop-target");
      // Opened, the list sits one row lower: the row under the same pointer is another one now.
      await tick(500);
      expect(view()).toBe("expanded");
      expect(row(0).classes()).toContain("drop-target");
      expect(row(1).classes()).not.toContain("drop-target");
    });

    it("lets a window change that a wait started finish before it handles the next event", async () => {
      await smallWindow();
      let release!: () => void;
      const held = new Promise<void>(resolve => { release = resolve; });
      const normal = mock.invoke.getMockImplementation()!;
      mock.invoke.mockImplementation(async (command: string, args?: unknown) => {
        if (command === "popup_inbox_commit") await held;
        return normal(command, args);
      });
      await onRow(1, "enter");
      await tick(500);
      expect(view()).toBe("expanded");
      // The window is still changing form: this drop waits for it instead of finding the page in between.
      await send("drop", BODY + 280, CONTENT.top + 250, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([]);
      release(); await tick(0);
      expect(mock.received).toEqual([{ id: "b", paths: ["/tmp/x.txt"], question: 0 }]);
    });

    it("hands a drop on the body to the open request, shows it with the window in front, and tells the user", async () => {
      await peek(0);
      await send("drop", BODY + 280, CONTENT.top + 250, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "a", paths: ["/tmp/x.txt"], question: 0 }]);
      expect(mock.focus).toHaveBeenCalledTimes(1);
      expect(visible("a")).toBe(true);
      expect(wrapper.find(".inbox-notice").text()).toBe("Added 1 file to “Request a”");
    });

    it("hands a drop on another request's row to that request, and switches to it", async () => {
      await peek(0);
      await send("drop", 120, rowY(2), ["/tmp/x.txt", "/tmp/y.txt"]);
      await tick(126 + 174);
      expect(mock.received).toEqual([{ id: "c", paths: ["/tmp/x.txt", "/tmp/y.txt"], question: 0 }]);
      expect(visible("c")).toBe(true);
      expect(visible("a")).toBe(false);
      expect(mock.focus).toHaveBeenCalledTimes(1);
    });

    it("hands a drop on the question the request is on, when the pointer is not on a card", async () => {
      await peek(0);
      mock.current.set("a", 1);
      await send("drop", BODY + 280, 560, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "a", paths: ["/tmp/x.txt"], question: 1 }]);
    });

    it("does not take files while a dialog has the window, and ends the drag", async () => {
      await peek(0);
      emit("popup-inbox-close", undefined); await flushPromises();
      expect(wrapper.find(".inbox-close-dialog").exists()).toBe(true);
      await onRow(1, "enter");
      expect(dragging()).toBe(false);
      expect(wrapper.findAll(".inbox-row.dwelling")).toHaveLength(0);
      await send("drop", 120, rowY(1), ["/tmp/x.txt"]);
      expect(mock.received).toEqual([]);
      await tick(1000);
      expect(visible("a")).toBe(true);
    });

    it("does not count the user dragging one of the request's own attachments out of the window", async () => {
      await peek(0);
      mock.draggingOut.add("a");
      await onRow(1, "enter");
      expect(wrapper.findAll(".inbox-row.dwelling")).toHaveLength(0);
      expect(wrapper.findAll(".inbox-row.drop-target")).toHaveLength(0);
      expect(dragging()).toBe(false);
      await send("drop", 120, rowY(1), ["/tmp/x.txt"]);
      expect(mock.received).toEqual([]);
      // The request is told the drag is over.
      expect(mock.draggingOut.has("a")).toBe(false);
    });

    it("lights a row under the pointer and shows no aim inside the request", async () => {
      await peek(0);
      await onRow(1, "enter");
      expect(row(1).classes()).toContain("drop-target");
      expect(aimOf("a")).toBe("none");
    });
  });

  describe("the questions of a request", () => {
    it("aims at the card under the pointer in the stacked layout, and at the request when between cards", async () => {
      mock.stacked.set("a", 3);
      await peek(0);
      await send("enter", BODY + 280, CONTENT.top + 20 + 150 + 60);
      expect(aimOf("a")).toBe("1");
      // The other requests, mounted but out of sight, show nothing.
      expect(aimOf("b")).toBe("none");
      await send("over", BODY + 280, CONTENT.top + 20 + 150 * 2 + 60);
      expect(aimOf("a")).toBe("2");
      // The gap between two cards, and the footer: still this request, but no particular question.
      await send("over", BODY + 280, CONTENT.top + 20 + 140);
      expect(aimOf("a")).toBe("request");
      await send("over", BODY + 280, 580);
      expect(aimOf("a")).toBe("request");
    });

    it("hands a drop on a card to that question, makes it the request's current one, and names it", async () => {
      mock.stacked.set("a", 3);
      await peek(0);
      await send("drop", BODY + 280, CONTENT.top + 20 + 150 * 2 + 60, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "a", paths: ["/tmp/x.txt"], question: 2 }]);
      expect(mock.current.get("a")).toBe(2);
      expect(wrapper.find(".inbox-notice").text()).toBe("Added 1 file to “Request a”, question 3");
    });

    it("hands a drop between cards to the current question", async () => {
      mock.stacked.set("a", 3);
      await peek(0);
      mock.current.set("a", 1);
      await send("drop", BODY + 280, 580, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "a", paths: ["/tmp/x.txt"], question: 1 }]);
    });

    it("hands a drop on another request's row to that request's current question", async () => {
      mock.stacked.set("a", 3); mock.stacked.set("b", 3);
      await peek(0);
      mock.current.set("b", 2);
      await send("drop", 120, rowY(1), ["/tmp/x.txt"]);
      await tick(126 + 174);
      expect(mock.received).toEqual([{ id: "b", paths: ["/tmp/x.txt"], question: 2 }]);
      expect(wrapper.find(".inbox-notice").text()).toBe("Added 1 file to “Request b”, question 3");
    });

    it("hands a drop on the small window's row to that request's current question", async () => {
      mock.stacked.set("b", 3);
      await smallWindow();
      mock.current.set("b", 1);
      await send("drop", 120, rowY(1), ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "b", paths: ["/tmp/x.txt"], question: 1 }]);
    });

    it("sends nothing to a card in a request that cannot take files", async () => {
      requests = [ask("a"), confirmation("b"), ask("c")];
      mock.stacked.set("b", 3);
      await smallWindow();
      await send("drop", 120, rowY(1), ["/tmp/x.txt"]);
      expect(mock.received).toEqual([]);
    });
  });

  describe("scrolling by the edges", () => {
    // A list longer than the window: 26 requests of 40 each, in a list that shows 520.
    const letters = "abcdefghijklmnopqrstuvwxyz".split("");
    async function list() {
      await start(...letters);
      const navigation = wrapper.find(".inbox-navigation").element;
      scroller(navigation, letters.length * ROW + 30, 520);
      arrange();
      return navigation;
    }
    const bottomOfList = 619, topOfList = 71;

    it("scrolls the list down while the pointer rests at its bottom edge, and stops where it ends", async () => {
      const navigation = await list();
      await send("enter", 120, bottomOfList);
      await tick(500);
      expect(navigation.scrollTop).toBeGreaterThan(200);
      await tick(2000);
      expect(navigation.scrollTop).toBe(letters.length * ROW + 30 - 520);
    });

    it("scrolls back up at the top edge", async () => {
      const navigation = await list();
      navigation.scrollTop = 300;
      await send("enter", 120, topOfList);
      await tick(400);
      expect(navigation.scrollTop).toBeLessThan(150);
      await tick(2000);
      expect(navigation.scrollTop).toBe(0);
    });

    it("goes faster the closer the pointer is to the edge", async () => {
      const navigation = await list();
      await send("enter", 120, 620 - 56 + 14);
      await tick(1000);
      const slowDistance = navigation.scrollTop;
      expect(slowDistance).toBeGreaterThan(0);
      await send("leave");
      navigation.scrollTop = 0;
      await send("enter", 120, bottomOfList);
      await tick(1000);
      expect(navigation.scrollTop).toBeGreaterThan(slowDistance * 3);
    });

    it("does not scroll in the middle, nor after the pointer leaves the edge or the window", async () => {
      const navigation = await list();
      await send("enter", 120, 350);
      await tick(1000);
      expect(navigation.scrollTop).toBe(0);
      await send("over", 120, bottomOfList);
      await tick(200);
      const there = navigation.scrollTop;
      expect(there).toBeGreaterThan(0);
      await send("over", 120, 350);
      await tick(500);
      expect(navigation.scrollTop).toBe(there);
      await send("over", 120, bottomOfList);
      await tick(200);
      await send("leave");
      const gone = navigation.scrollTop;
      await tick(500);
      expect(navigation.scrollTop).toBe(gone);
    });

    it("makes the row that slides under a resting pointer the target, and waits for it like any other", async () => {
      const navigation = await list();
      const y = 610;
      const under = () => Math.floor((y - ROW_EXPANDED + navigation.scrollTop) / ROW);
      await send("enter", 120, y);
      expect(row(under()).classes()).toContain("drop-target");
      await tick(300);
      // The pointer did not move, but the list did: the target is the row that is under it now, and only that row.
      expect(navigation.scrollTop).toBeGreaterThan(100);
      expect(wrapper.findAll(".inbox-row.drop-target")).toHaveLength(1);
      expect(row(under()).classes()).toContain("drop-target");
      expect(row(under()).classes()).toContain("dwelling");
    });

    it("hands a drop to the row that slid under the pointer", async () => {
      const navigation = await list();
      await send("enter", 120, 610);
      await tick(400);
      const index = Math.floor((610 - ROW_EXPANDED + navigation.scrollTop) / ROW);
      await send("drop", 120, 610, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: letters[index], paths: ["/tmp/x.txt"], question: 0 }]);
    });

    it("scrolls the small window's list as well: a window of limited height lists more requests than fit", async () => {
      await smallWindow();
      const navigation = wrapper.find(".inbox-navigation").element;
      scroller(navigation, 1000, 120);
      arrange();
      await send("enter", 120, 178);
      await tick(300);
      expect(navigation.scrollTop).toBeGreaterThan(50);
    });

    it("scrolls the body at its bottom edge and aims at the card that comes under the pointer", async () => {
      mock.stacked.set("a", 8);
      await peek(0);
      const content = wrapper.find('[data-inbox-request="a"] .content').element;
      scroller(content, 60 + 150 * 8, 500);
      arrange();
      const y = CONTENT.top + CONTENT.height - 4;
      const under = () => {
        const offset = y - (CONTENT.top + 20) + content.scrollTop;
        return offset % 150 < 130 ? String(Math.floor(offset / 150)) : "request";
      };
      await send("enter", BODY + 280, y);
      expect(aimOf("a")).toBe("3");
      await tick(600);
      expect(content.scrollTop).toBeGreaterThan(300);
      // The pointer did not move, but the cards did: the aim is the card that is under it now.
      expect(aimOf("a")).toBe(under());
      expect(aimOf("a")).not.toBe("3");
    });

    it("scrolls the body when the pointer is above it, over the top of the window", async () => {
      mock.stacked.set("a", 8);
      await peek(0);
      const content = wrapper.find('[data-inbox-request="a"] .content').element;
      scroller(content, 60 + 150 * 8, 500);
      content.scrollTop = 300;
      arrange();
      await send("enter", BODY + 280, 10);
      await tick(500);
      expect(content.scrollTop).toBeLessThan(300);
    });
  });

  describe("with the fixed canvas of the macOS window", () => {
    beforeEach(() => { canvas = true; });

    it("finds the row under the pointer in the small window, which is much smaller than the canvas it sits on", async () => {
      await smallWindow();
      await onRow(1, "enter");
      // A y that was not corrected for the canvas sticking out below the window would read far below every row.
      expect(row(1).classes()).toContain("dwelling");
      expect(row(0).classes()).not.toContain("dwelling");
      expect(row(2).classes()).not.toContain("dwelling");
      await tick(500);
      expect(visible("b")).toBe(true);
    });

    it("drops on the row under the pointer in the small window", async () => {
      await smallWindow();
      await send("drop", LEFT + 120, rowY(0), ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "a", paths: ["/tmp/x.txt"], question: 0 }]);
    });

    it("needs no correction once the window has settled at its full size", async () => {
      await smallWindow();
      await row(0).trigger("click"); await flushPromises();
      arrange();
      expect(view()).toBe("expanded");
      await onRow(2, "enter");
      expect(row(2).classes()).toContain("dwelling");
      await send("drop", LEFT + BODY + 280, CONTENT.top + 250, ["/tmp/x.txt"]);
      expect(mock.received).toEqual([{ id: "a", paths: ["/tmp/x.txt"], question: 0 }]);
    });
  });

  describe("leaving AskHuman while files are dragged over the window", () => {
    it("does not shrink a peek: the drag is the user working with the window, wherever the focus is", async () => {
      await peek(0);
      emit("popup-app-active", false);
      await onBody("enter");
      mock.invoke.mockClear();
      await tick(1000);
      expect(layouts()).toEqual([]);
      expect(view()).toBe("expanded");
    });

    it("still shrinks a peek the user leaves when no files are being dragged", async () => {
      await peek(0);
      emit("popup-app-active", false);
      await tick(200);
      expect(small()).toBe(true);
    });
  });

  describe("when the round ends", () => {
    it("does not carry a drag into the next round when the last request ends under it", async () => {
      await start("a");
      await onBody("enter");
      expect(dragging()).toBe(true);
      emit("popup-inbox-terminal", { requestId: "a", winner: "slack" });
      await tick(3000);
      expect(wrapper.findAll(".inbox-request")).toHaveLength(0);
      expect(dragging()).toBe(false);
    });
  });

  describe("when the window goes away", () => {
    it("forgets a wait that was running", async () => {
      await smallWindow();
      await onRow(1, "enter");
      await tick(200);
      wrapper.unmount();
      mock.invoke.mockClear();
      await tick(3000);
      expect(layouts()).toEqual([]);
      expect(mock.drag).toBeNull();
    });
  });
});
