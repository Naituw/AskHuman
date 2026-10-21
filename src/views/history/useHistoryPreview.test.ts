import { defineComponent, h, ref, shallowRef, type Ref } from "vue";
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryEntry } from "../../lib/types";

const mocks = vi.hoisted(() => ({
  content: { kind: "text", text: "hello" } as Record<string, unknown>,
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  closeWindow: vi.fn(async () => {}),
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
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: () => ({ close: mocks.closeWindow }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn(async () => {}) }));
vi.mock("../../lib/ipc", () => ({
  fileIconDataUrl: vi.fn(async () => ""),
  openPath: vi.fn(async () => {}),
  readImageDataUrl: vi.fn(async () => ""),
  showAttachmentMenu: vi.fn(async () => {}),
  localImageCreateScope: vi.fn(async () => "scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
}));
import { localImagePrepare, localImageReleaseScope } from "../../lib/ipc";
import { useHistoryPreview, type HistoryPreview } from "./useHistoryPreview";

function entryOf(id: string, files: string[] = [], replyImages: string[][] = []): HistoryEntry {
  return {
    id, timestampMs: 1, project: "", source: "", channel: "popup", action: "send", isMarkdown: false,
    message: { text: "", files: files.map(path => ({ path, name: path.split("/").pop()!, size: 1, isImage: false })) },
    questions: replyImages.map(() => ({ message: "question", predefinedOptions: [] })),
    answers: replyImages.map(images => ({ selectedOptions: [], userInput: null, images, files: [] })),
  };
}

let wrappers: VueWrapper[] = [];
function host(entry: Ref<HistoryEntry | null>, blocked = ref(false)) {
  let preview!: HistoryPreview;
  const wrapper = mount(defineComponent({
    setup() { preview = useHistoryPreview({ entry, blocked }); return () => h("div"); },
  }), { attachTo: document.body });
  wrappers.push(wrapper);
  return { preview, wrapper };
}
async function settle() { for (let i = 0; i < 40; i++) await Promise.resolve(); }
function layoutCalls(requestId?: string) {
  return mocks.invoke.mock.calls.filter(([command, args]) =>
    command === "popup_preview_layout" && (requestId === undefined || args.requestId === requestId));
}
const last = <T,>(items: T[]) => items[items.length - 1];
function press(key: string) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  window.dispatchEvent(event);
  return event;
}

describe("history attachment preview controller", () => {
  beforeEach(() => {
    vi.clearAllMocks(); mocks.listeners.clear(); document.body.replaceChildren();
    mocks.content = { kind: "text", text: "hello" };
    mocks.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "popup_preview_prepare" || command === "popup_preview_layout") {
        return { revision: args.version, side: args.open ? "right" : "closed", mainWidth: 600, mainHeight: 620 };
      }
      if (command === "popup_preview_read") {
        return { requestId: args.requestId, index: args.index, generation: args.generation, content: mocks.content };
      }
      return undefined;
    });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
    vi.stubGlobal("Image", class {
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      naturalWidth = 40;
      naturalHeight = 30;
      set src(value: string) { if (value) queueMicrotask(() => this.onload?.()); }
    });
  });
  afterEach(() => { for (const wrapper of wrappers) wrapper.unmount(); wrappers = []; });

  it("previews the entry on screen under a history owner id and reads its files by that id", async () => {
    const { preview } = host(ref(entryOf("a", ["/tmp/a.md", "/tmp/b.md"])));
    expect(preview.ownerId.value).toBe("history:a");
    preview.attach.selectFile(1); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_prepare", expect.objectContaining({ requestId: "history:a", open: true }));
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_layout", expect.objectContaining({ requestId: "history:a", open: true }));
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_read", expect.objectContaining({ requestId: "history:a", index: 1 }));
    expect(preview.attach.previewLayout.value.side).toBe("right");
    expect(preview.attach.previewFile.value?.path).toBe("/tmp/b.md");
  });

  it("takes layouts from the History window's own event, never the popup's", async () => {
    host(ref(null)); await settle();
    expect(mocks.listeners.has("history-preview-layout")).toBe(true);
    expect(mocks.listeners.has("popup-preview-layout")).toBe(false);
    expect(mocks.listeners.has("popup-preview-native-key")).toBe(true);
  });

  it("hands the panel an owner-bound context without find", () => {
    const { preview } = host(ref(entryOf("a", ["/tmp/a.md"])));
    expect(preview.panel.ownerId.value).toBe("history:a");
    expect(preview.panel.findSupported).toBe(false);
    expect(preview.panel.findActive.value).toBe(false);
    expect(preview.panel.popupActive.value).toBe(true);
  });

  it("collapses under the old owner id before another entry takes over, which starts closed", async () => {
    const entry = shallowRef<HistoryEntry | null>(entryOf("a", ["/tmp/a.md"]));
    const { preview } = host(entry);
    preview.attach.selectFile(0); await settle();
    expect(preview.attach.previewOpen.value).toBe(true);
    entry.value = entryOf("b", ["/tmp/c.md"]);
    expect(preview.attach.previewOpen.value).toBe(false);
    expect(preview.ownerId.value).toBe("history:b");
    await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_prepare", expect.objectContaining({ requestId: "history:a", open: false }));
    expect(last(layoutCalls("history:a"))[1]).toMatchObject({ open: false });
    expect(layoutCalls("history:b")).toHaveLength(0);
    expect(preview.attach.previewLayout.value.side).toBe("closed");
  });

  it("collapses when the entry on screen goes away", async () => {
    const entry = shallowRef<HistoryEntry | null>(entryOf("a", ["/tmp/a.md"]));
    const { preview } = host(entry);
    preview.attach.selectFile(0); await settle();
    entry.value = null;
    expect(preview.attach.previewOpen.value).toBe(false);
    expect(preview.ownerId.value).toBe("history:");
    await settle();
    expect(last(layoutCalls("history:a"))[1]).toMatchObject({ open: false });
  });

  it("keeps an open preview where it is when a reload brings the same entry again", async () => {
    const entry = shallowRef<HistoryEntry | null>(entryOf("a", ["/tmp/a.md", "/tmp/b.md"]));
    const { preview } = host(entry);
    preview.attach.selectFile(1); await settle();
    const calls = mocks.invoke.mock.calls.length;
    entry.value = entryOf("a", ["/tmp/a.md", "/tmp/b.md"]); await settle();
    expect(preview.attach.previewIndex.value).toBe(1);
    expect(preview.attach.previewFile.value?.path).toBe("/tmp/b.md");
    expect(mocks.invoke.mock.calls.length).toBe(calls);
  });

  it("shows a reply image through its display URL and keeps it selected across a reload", async () => {
    mocks.content = { kind: "text", text: "unused" };
    const entry = shallowRef<HistoryEntry | null>(entryOf("a", [], [["/tmp/x.png", "/tmp/y.png"]]));
    const { preview } = host(entry);
    preview.attach.selectReplyImage(0, 1); await settle();
    expect(preview.attach.isReplyPreview.value).toBe(true);
    expect(preview.attach.previewName.value).toBe("y.png");
    expect(localImagePrepare).toHaveBeenCalledWith("scope", "/tmp/y.png");
    expect(preview.attach.panelContent.value).toEqual({ kind: "image", url: "askhuman-image://localhost//tmp/y.png", width: 40, height: 30 });
    // No file read for a reply image: its path never leaves the history record.
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "popup_preview_read")).toHaveLength(0);

    entry.value = entryOf("a", [], [["/tmp/x.png", "/tmp/y.png"]]); await settle();
    expect(preview.attach.isReplyPreview.value).toBe(true);
    expect(preview.attach.selectedReplyImage.value).toBe(1);
    expect(preview.attach.previewName.value).toBe("y.png");

    entry.value = entryOf("b", [], [["/tmp/z.png"]]); await settle();
    expect(preview.attach.previewOpen.value).toBe(false);
    expect(preview.attach.isReplyPreview.value).toBe(false);
    // The display resources of the entry that left are released with it.
    expect(localImageReleaseScope).toHaveBeenCalledWith("scope");
  });

  it("closes the panel with Escape wherever the focus is, and leaves keys alone while an overlay is up", async () => {
    const blocked = ref(false);
    const { preview } = host(ref(entryOf("a", ["/tmp/a.md"])), blocked);
    preview.attach.selectFile(0); await settle();
    blocked.value = true;
    expect(press("Escape").defaultPrevented).toBe(false);
    expect(preview.attach.previewOpen.value).toBe(true);
    blocked.value = false;
    expect(press("Escape").defaultPrevented).toBe(true);
    expect(preview.attach.previewOpen.value).toBe(false);
    expect(press("Escape").defaultPrevented).toBe(false);
  });

  it("answers the keys the native system preview forwards, for its own entry and file only", async () => {
    const { preview } = host(ref(entryOf("a", ["/tmp/a.pdf", "/tmp/b.pdf"])));
    mocks.content = { kind: "native", imageCount: null };
    preview.attach.selectFile(0); await settle();
    expect(preview.attach.panelContent.value?.kind).toBe("native");
    const native = mocks.listeners.get("popup-preview-native-key")!;
    native({ payload: { requestId: "history:other", index: 0, key: "Escape", metaKey: false } });
    native({ payload: { requestId: "history:a", index: 1, key: "Escape", metaKey: false } });
    native({ payload: { requestId: "history:a", index: 0, key: "interaction", metaKey: false } });
    expect(preview.attach.previewOpen.value).toBe(true);
    native({ payload: { requestId: "history:a", index: 0, key: "w", metaKey: true } });
    expect(mocks.closeWindow).toHaveBeenCalledTimes(1);
    native({ payload: { requestId: "history:a", index: 0, key: "Escape", metaKey: false } });
    expect(preview.attach.previewOpen.value).toBe(false);
  });

  it("ignores forwarded keys when the shown file is not a native preview", async () => {
    const { preview } = host(ref(entryOf("a", ["/tmp/a.md"])));
    preview.attach.selectFile(0); await settle();
    mocks.listeners.get("popup-preview-native-key")!({ payload: { requestId: "history:a", index: 0, key: "Escape", metaKey: false } });
    expect(preview.attach.previewOpen.value).toBe(true);
  });

  it("drops its listeners and window keys when the window goes", async () => {
    const { preview, wrapper } = host(ref(entryOf("a", ["/tmp/a.md"]))); await settle();
    expect(mocks.listeners.size).toBeGreaterThan(0);
    wrapper.unmount(); wrappers = [];
    expect(mocks.listeners.size).toBe(0);
    preview.attach.selectFile(0);
    expect(press("Escape").defaultPrevented).toBe(false);
  });
});
