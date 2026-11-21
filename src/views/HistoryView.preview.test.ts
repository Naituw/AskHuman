import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryEntry } from "../lib/types";

const mocks = vi.hoisted(() => ({
  entries: [] as unknown[],
  invoke: vi.fn(),
  link: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, handler);
    return () => { mocks.listeners.delete(name); };
  }),
}));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: () => ({ close: vi.fn(async () => {}) }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn(async () => {}) }));
vi.mock("../lib/ipc", () => ({
  clearAllHistory: vi.fn(async () => 0),
  deleteHistoryEntries: vi.fn(async () => 0),
  getHistory: vi.fn(async () => mocks.entries),
  getHistoryProjects: vi.fn(async () => []),
  historyInit: vi.fn(async () => ({ theme: "light", lang: "en", project: "", projectName: "" })),
  resolveHistorySessionTitles: vi.fn(async () => []),
  fileIconDataUrl: vi.fn(async () => ""),
  openPath: vi.fn(async () => {}),
  readImageDataUrl: vi.fn(async () => ""),
  showAttachmentMenu: vi.fn(async () => {}),
  popupPreviewLink: mocks.link,
  localImageCreateScope: vi.fn(async () => "scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
}));
import HistoryView from "./HistoryView.vue";
import { i18n } from "../i18n";
import { markdownReady } from "../lib/markdown";
import { openPath } from "../lib/ipc";
import { desktopPlatform } from "../lib/platform";

function entryOf(id: string, file: string): HistoryEntry {
  return {
    id, timestampMs: 1_700_000_000_000, project: "", source: "", channel: "popup", action: "send", isMarkdown: false,
    message: { text: `message ${id}`, files: [{ path: file, name: file.split("/").pop()!, size: 5, isImage: false }] },
    questions: [], answers: [],
  };
}

let wrapper: VueWrapper | undefined;
async function mountHistory() {
  wrapper = mount(HistoryView, { attachTo: document.body, global: { plugins: [i18n] } });
  await flushPromises(); await flushPromises();
  return wrapper;
}
function publish(layout: { revision: number; side: "closed" | "right" | "inside"; mainWidth: number }) {
  mocks.listeners.get("history-preview-layout")!({ payload: { mainHeight: 620, ...layout } });
  return nextTick();
}
function click(target: Element, detail = 1) {
  target.dispatchEvent(new MouseEvent("click", { bubbles: true, detail }));
}
function press(key: string) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  window.dispatchEvent(event);
  return event;
}
const layoutCalls = () => mocks.invoke.mock.calls.filter(([command]) => command === "popup_preview_layout");
const last = <T,>(items: T[]) => items[items.length - 1];

describe("history window attachment panel", () => {
  beforeEach(() => {
    vi.clearAllMocks(); mocks.listeners.clear(); document.body.replaceChildren();
    i18n.global.locale.value = "en";
    mocks.entries = [entryOf("e1", "/tmp/one.md"), entryOf("e2", "/tmp/two.md")];
    mocks.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "popup_preview_prepare" || command === "popup_preview_layout") {
        return { revision: args.version, side: args.open ? "right" : "closed", mainWidth: 600, mainHeight: 620 };
      }
      if (command === "popup_preview_read") {
        return { requestId: args.requestId, index: args.index, generation: args.generation, content: { kind: "text", text: "file body" } };
      }
      return undefined;
    });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  });
  afterEach(() => { wrapper?.unmount(); wrapper = undefined; });

  it("keeps a single column until a file is opened, then splits the window into main, divider and panel", async () => {
    const view = await mountHistory();
    expect(view.find(".attachment-preview").exists()).toBe(false);
    expect(view.find(".attachment-preview-divider").exists()).toBe(false);
    expect(view.get(".history-shell").attributes("style")).toContain("--history-main-width: 100vw");

    click(view.get(".attachment").element);
    await flushPromises();
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_layout", expect.objectContaining({ requestId: "history:e1", open: true }));
    // The geometry the window's controller answers with is what the page lays out by (here the mock's 600).
    expect(view.get(".history-shell").attributes("style")).toContain("grid-template-columns: 600px 6px minmax(0, 1fr)");

    // A later publication (the user dragged the divider, the window was resized) replaces it.
    await publish({ revision: 50, side: "right", mainWidth: 640 });
    const style = view.get(".history-shell").attributes("style")!;
    expect(style).toContain("grid-template-columns: 640px 6px minmax(0, 1fr)");
    expect(style).toContain("--history-main-width: 640px");
    expect(view.find(".attachment-preview-divider").exists()).toBe(true);
    await flushPromises();
    expect(view.get(".attachment-preview-name").text()).toBe("one.md");
    expect(view.get(".attachment-preview-text").text()).toBe("file body");
    expect(view.get(".history").classes()).not.toContain("list-collapsed");

    click(view.get(".attachment").element);
    await flushPromises();
    await publish({ revision: 51, side: "closed", mainWidth: 900 });
    expect(view.find(".attachment-preview").exists()).toBe(false);
    expect(view.find(".attachment-preview-divider").exists()).toBe(false);
  });

  it("steps the entry list aside when the panel sits inside a narrow window", async () => {
    const view = await mountHistory();
    click(view.get(".attachment").element); await flushPromises();
    await publish({ revision: 50, side: "inside", mainWidth: 300 });
    expect(view.get(".history").classes()).toContain("list-collapsed");
    expect(view.get(".history-shell").attributes("style")).toContain("--history-main-width: 300px");
    await publish({ revision: 51, side: "right", mainWidth: 600 });
    expect(view.get(".history").classes()).not.toContain("list-collapsed");
  });

  it("closes the panel when another entry is chosen and shows nothing of the old one", async () => {
    const view = await mountHistory();
    click(view.get(".attachment").element); await flushPromises();
    await publish({ revision: 50, side: "right", mainWidth: 640 });
    expect(view.find(".attachment-preview").exists()).toBe(true);
    click(view.findAll(".entry")[1].element);
    await flushPromises();
    expect(view.find(".attachment-preview").exists()).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_prepare", expect.objectContaining({ requestId: "history:e1", open: false }));
    expect(view.get(".att-name").text()).toBe("two.md");
    expect(view.get(".attachment").classes()).not.toContain("selected");
  });

  it("closes the panel with Escape, but not while the delete confirmation is up", async () => {
    const view = await mountHistory();
    click(view.get(".attachment").element); await flushPromises();
    await publish({ revision: 50, side: "right", mainWidth: 640 });

    await view.get(".clear-btn").trigger("click");
    await view.findAll(".clear-menu button")[0].trigger("click");
    expect(view.find(".overlay").exists()).toBe(true);
    expect(press("Escape").defaultPrevented).toBe(false);
    expect(view.find(".attachment-preview").exists()).toBe(true);

    await view.get(".btn-ghost").trigger("click");
    expect(view.find(".overlay").exists()).toBe(false);
    const before = layoutCalls().length;
    expect(press("Escape").defaultPrevented).toBe(true);
    await flushPromises();
    expect(layoutCalls().length).toBeGreaterThan(before);
    expect(last(layoutCalls())[1]).toMatchObject({ requestId: "history:e1", open: false });
  });

  it("hides the find entry point: the history panel has no in-page find", async () => {
    const view = await mountHistory();
    click(view.get(".attachment").element); await flushPromises();
    await publish({ revision: 50, side: "right", mainWidth: 640 });
    expect(view.find(".attachment-preview-find").exists()).toBe(false);
    expect(view.find(".attachment-preview-open").exists()).toBe(true);
  });
});

describe("history window and the files an entry's text links to", () => {
  beforeAll(() => markdownReady);

  const linked = (path: string) => ({ path, name: path.split("/").pop()!, size: 9, isImage: false });
  const text = "see [notes](/work/docs/notes.md:12) and [again](/work/docs/notes.md:40) and ![shot](/tmp/shot.png)";

  beforeEach(() => {
    vi.clearAllMocks(); mocks.listeners.clear(); document.body.replaceChildren();
    i18n.global.locale.value = "en";
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
    mocks.entries = [{ ...entryOf("e1", "/tmp/one.md"), isMarkdown: true, message: { text, files: [linked("/tmp/one.md")] } }, entryOf("e2", "/tmp/two.md")];
    // The backend registers a linked file after the entry's one attachment.
    mocks.link.mockImplementation(async (_owner: string, path: string) => ({ kind: "file", index: 1, file: linked(path) }));
    mocks.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "popup_preview_prepare" || command === "popup_preview_layout") {
        return { revision: args.version, side: args.open ? "right" : "closed", mainWidth: 600, mainHeight: 620 };
      }
      if (command === "popup_preview_read") {
        return { requestId: args.requestId, index: args.index, generation: args.generation, content: { kind: "text", text: "file body" } };
      }
      return undefined;
    });
  });
  afterEach(() => { wrapper?.unmount(); wrapper = undefined; vi.unstubAllGlobals(); });

  const links = (view: VueWrapper) => view.findAll(".markdown-body a");
  // A real click can be cancelled; the shared `click` above cannot, and jsdom would try to navigate.
  function clickOn(target: Element, init: MouseEventInit = {}) {
    const event = new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1, ...init });
    target.dispatchEvent(event);
    return event;
  }

  it("opens in the panel on a click, selected while it is on show and not mistaken for an attachment", async () => {
    const view = await mountHistory();
    clickOn(links(view)[0].element); await flushPromises();
    expect(mocks.link).toHaveBeenCalledExactlyOnceWith("history:e1", "/work/docs/notes.md");
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_layout", expect.objectContaining({ requestId: "history:e1", open: true }));
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_read", expect.objectContaining({ requestId: "history:e1", index: 1 }));
    expect(openPath).not.toHaveBeenCalled();
    await publish({ revision: 50, side: "right", mainWidth: 640 });
    expect(view.get(".attachment-preview-name").text()).toBe("notes.md");
    // A file on its own: nothing to step through.
    expect(view.find(".attachment-preview-count").exists()).toBe(false);
    // Every link to the file on show is selected, whichever line it names; the chip is not.
    expect(links(view).map(link => link.classes("is-active"))).toEqual([true, true]);
    expect(view.get(".attachment").classes()).not.toContain("selected");

    // The chip takes the panel back to the attachments' own navigation.
    click(view.get(".attachment").element); await flushPromises();
    expect(view.get(".attachment-preview-name").text()).toBe("one.md");
    expect(view.get(".attachment-preview-count").text()).toBe("1 / 1");
    expect(links(view).some(link => link.classes("is-active"))).toBe(false);
    expect(view.get(".attachment").classes()).toContain("selected");
  });

  it("collapses on a second click of the selected link, and with the entry that owns it", async () => {
    const view = await mountHistory();
    clickOn(links(view)[1].element); await flushPromises();
    await publish({ revision: 50, side: "right", mainWidth: 640 });
    expect(view.find(".attachment-preview").exists()).toBe(true);
    clickOn(links(view)[1].element); await flushPromises();
    await publish({ revision: 51, side: "closed", mainWidth: 900 });
    expect(view.find(".attachment-preview").exists()).toBe(false);
    expect(links(view)[1].classes("is-active")).toBe(false);

    clickOn(links(view)[0].element); await flushPromises();
    await publish({ revision: 52, side: "right", mainWidth: 640 });
    click(view.findAll(".entry")[1].element); await flushPromises();
    expect(view.find(".attachment-preview").exists()).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_prepare", expect.objectContaining({ requestId: "history:e1", open: false }));
    click(view.findAll(".entry")[0].element); await flushPromises();
    expect(view.find(".attachment-preview").exists()).toBe(false);
    expect(links(view).some(link => link.classes("is-active"))).toBe(false);
  });

  it("opens the attachment menu for the linked file on a right click", async () => {
    const view = await mountHistory();
    const menu = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    links(view)[0].element.dispatchEvent(menu); await flushPromises();
    expect(menu.defaultPrevented).toBe(true);
    expect(mocks.link).toHaveBeenCalledWith("history:e1", "/work/docs/notes.md");
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_menu", { requestId: "history:e1", index: 1 });
    // The menu is not a selection: the panel stays shut.
    expect(view.find(".attachment-preview").exists()).toBe(false);
  });

  it("previews a local image of the text in the panel and selects it", async () => {
    const view = await mountHistory();
    const image = view.get(".markdown-body img");
    clickOn(image.element); await flushPromises();
    expect(mocks.link).toHaveBeenCalledExactlyOnceWith("history:e1", "/tmp/shot.png");
    expect(openPath).not.toHaveBeenCalled();
    await publish({ revision: 50, side: "right", mainWidth: 640 });
    expect(view.get(".attachment-preview-name").text()).toBe("shot.png");
    expect(view.get(".markdown-body img").classes("is-active")).toBe(true);
  });

  it("leaves a ⌘ / Ctrl + click on a link to the system, with no panel", async () => {
    const view = await mountHistory();
    const mac = desktopPlatform === "mac";
    const event = clickOn(links(view)[0].element, { metaKey: mac, ctrlKey: !mac });
    await flushPromises();
    expect(event.defaultPrevented).toBe(true);
    expect(openPath).toHaveBeenCalledExactlyOnceWith("/work/docs/notes.md");
    expect(mocks.link).not.toHaveBeenCalled();
    expect(view.find(".attachment-preview").exists()).toBe(false);
  });
});
