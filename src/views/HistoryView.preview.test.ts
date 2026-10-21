import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryEntry } from "../lib/types";

const mocks = vi.hoisted(() => ({
  entries: [] as unknown[],
  invoke: vi.fn(),
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
  localImageCreateScope: vi.fn(async () => "scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
}));
import HistoryView from "./HistoryView.vue";
import { i18n } from "../i18n";

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
