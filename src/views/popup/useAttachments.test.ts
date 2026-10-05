import { computed, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileAttachment } from "../../lib/types";
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async (_command: string, args: { open: boolean; version: number }) => ({
    revision: args.version, side: args.open ? "right" : "closed", mainWidth: 560, mainHeight: 620,
  })),
  openPath: vi.fn(async () => {}),
  startDrag: vi.fn(async () => {}),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../lib/ipc", () => ({
  fileIconDataUrl: vi.fn(async () => ""), openPath: mocks.openPath,
  readImageDataUrl: vi.fn(async () => ""), showAttachmentMenu: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
  mocks.listeners.set(name, handler); return () => mocks.listeners.delete(name);
}) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: mocks.startDrag }));
import { useAttachments } from "./useAttachments";
const files: FileAttachment[] = [0, 1, 2].map(i => ({ path: `/tmp/${i}.patch`, name: `${i}.patch`, size: 128, isImage: false }));
function setup(attachments = files) {
  const requestId = ref("request-1");
  const state = useAttachments({ attachments: computed(() => attachments), requestId: computed(() => requestId.value) });
  const elements = files.map((_, i) => {
    const el = document.createElement("div"); el.className = "attachment"; el.tabIndex = 0;
    el.dataset.attachmentIndex = String(i); document.body.appendChild(el); state.setAttRef(el, i); return el;
  });
  return { state, elements, requestId };
}
function key(el: HTMLElement, value: string, extra: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, ...extra });
  el.dispatchEvent(event); return event;
}
async function settle() { for (let i=0; i<30; i++) await Promise.resolve(); }
describe("same-window attachment activation", () => {
  beforeEach(() => {
    vi.clearAllMocks(); mocks.listeners.clear(); document.body.replaceChildren();
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  });
  it("expands once while switching files, then toggles closed without opening an external app", async () => {
    const { state } = setup(); state.selectFile(0); await settle();
    state.selectFile(1); state.selectFile(2); await settle();
    expect(state.selectedFile.value).toBe(2); expect(mocks.invoke.mock.calls.filter(([command]) => command === "popup_preview_layout")).toHaveLength(1);
    state.selectFile(2); await settle();
    expect(state.selectedFile.value).toBeNull();
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_layout", expect.objectContaining({ open: false }));
    expect(mocks.openPath).not.toHaveBeenCalled();
  });
  it("routes arrows to attachment focus while leaving the body and editor native", () => {
    const { state, elements } = setup(); state.selectFile(0);
    expect(state.handleAttachmentKey(key(elements[0], "ArrowRight"))).toBe(true);
    expect(state.selectedFile.value).toBe(1); expect(document.activeElement).toBe(elements[1]);
    const body = document.createElement("div"); body.className = "attachment-preview-body";
    const editor = document.createElement("textarea");
    expect(state.handleAttachmentKey(key(body, "ArrowRight"))).toBe(false);
    expect(state.handleAttachmentKey(key(editor, " "))).toBe(false);
    expect(state.selectedFile.value).toBe(1);
  });
  it("keeps reading active while answering and closes with Escape without stealing focus", () => {
    const { state } = setup(); state.selectFile(0);
    const editor = document.createElement("textarea"); document.body.appendChild(editor); editor.focus();
    state.onBackgroundClick({ target: editor } as unknown as MouseEvent);
    expect(state.selectedFile.value).toBe(0);
    expect(state.handleAttachmentKey(key(editor, "Escape"))).toBe(true);
    expect(state.selectedFile.value).toBeNull(); expect(document.activeElement).toBe(editor);
  });
  it("leaves IME and modified keys alone", () => {
    const { state, elements } = setup(); state.selectFile(0);
    expect(state.handleAttachmentKey(key(elements[0], "Escape", { isComposing: true }))).toBe(false);
    expect(state.handleAttachmentKey(key(elements[0], "ArrowRight", { metaKey: true }))).toBe(false);
    expect(state.selectedFile.value).toBe(0);
  });
  it("does not toggle closed on the second click of a double click", () => {
    const { state } = setup(); state.selectFile(0, new MouseEvent("click", { detail: 1 }));
    state.selectFile(0, new MouseEvent("click", { detail: 2 })); state.openFile(files[0]);
    expect(state.selectedFile.value).toBe(0); expect(mocks.openPath).toHaveBeenCalledWith(files[0].path);
  });
  it("binds native actions to the request and selected original attachment", async () => {
    const { state } = setup(); state.selectFile(2);
    await state.revealFile(); state.showPreviewMenu();
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_reveal", { requestId: "request-1", index: 2 });
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_menu", { requestId: "request-1", index: 2 });
    state.openFile(files[2]);
    expect(mocks.openPath).toHaveBeenCalledWith(files[2].path);
  });
  it("shows native action failures only for the active request and attachment", async () => {
    const { state } = setup(); await state.initAttachmentPreviewListeners(); state.showPreview(1);
    const error = mocks.listeners.get("popup-preview-action-failed")!;
    error({ payload: { requestId: "old-request", index: 1 } });
    error({ payload: { requestId: "request-1", index: 0 } });
    expect(state.previewActionError.value).toBe(false);
    error({ payload: { requestId: "request-1", index: 1 } });
    expect(state.previewActionError.value).toBe(true);
    state.showPreview(2); await settle(); expect(state.previewActionError.value).toBe(false);
    state.disposeAttachments(); expect(mocks.listeners.has("popup-preview-action-failed")).toBe(false);
  });
  it("drops queued geometry work when disposed and uses a PNG fallback for drag-out", async () => {
    const { state } = setup(); state.selectFile(0); state.disposeAttachments(); await settle();
    expect(mocks.invoke).not.toHaveBeenCalled();
    state.onAttachmentDragStart(files[0], new Event("dragstart", { cancelable: true }) as DragEvent);
    expect(mocks.startDrag).toHaveBeenCalledWith(expect.objectContaining({ item: [files[0].path], icon: expect.stringContaining("data:image/png;base64,") }), expect.any(Function));
  });
  it("uses the reading mode for the primary action while Enter and double click keep the source", async () => {
    const markdown = [{ ...files[0], path: "/tmp/报告 space.MD", name: "报告 space.MD" }];
    const { state, elements } = setup(markdown);
    state.showPreview(0); await settle();
    state.previewContent.value = { kind: "markdown", text: "# Title", html: "<h1>Title</h1>" };
    state.openPreviewFile(); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_open_browser", { requestId: "request-1", index: 0 });
    expect(mocks.openPath).not.toHaveBeenCalled();
    state.currentReadingState.value!.raw = true;
    state.openPreviewFile();
    expect(mocks.openPath).toHaveBeenLastCalledWith(markdown[0].path);
    state.currentReadingState.value!.raw = false;
    mocks.openPath.mockClear();
    state.handleAttachmentKey(key(elements[0], "Enter"));
    state.selectFile(0, new MouseEvent("click", { detail: 2 })); state.openFile(markdown[0]);
    expect(mocks.openPath).toHaveBeenCalledTimes(2);
    expect(state.primaryBrowser.value).toBe(true);
    state.disposeAttachments();
  });
  it("opens the browser menu action even in source mode and ignores stale menu events", async () => {
    const { state } = setup(); await state.initAttachmentPreviewListeners(); state.showPreview(1); await settle();
    state.currentReadingState.value!.raw = true;
    const event = mocks.listeners.get("popup-preview-open-browser")!;
    event({ payload: { requestId: "old", index: 1 } });
    event({ payload: { requestId: "request-1", index: 99 } });
    expect(mocks.invoke.mock.calls.filter(([cmd]) => cmd === "popup_preview_open_browser")).toHaveLength(0);
    event({ payload: { requestId: "request-1", index: 1 } }); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_open_browser", { requestId: "request-1", index: 1 });
    expect(state.currentReadingState.value!.raw).toBe(true);
    state.disposeAttachments(); expect(mocks.listeners.has("popup-preview-open-browser")).toBe(false);
  });
  it.each(["limit", "browser launch failed"])("reports %s without changing attachment, falling back or issuing duplicate opens", async (failure) => {
    const { state } = setup(); state.showPreview(0); await settle();
    let reject!: (reason: string) => void;
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command, args) => command === "popup_preview_open_browser"
      ? new Promise((_, fail) => { reject = fail; }) : original(command, args));
    const opening = state.openBrowser(2); void state.openBrowser(2);
    expect(state.browserOpening.value).toBe(true);
    expect(mocks.invoke.mock.calls.filter(([cmd]) => cmd === "popup_preview_open_browser")).toHaveLength(1);
    reject(failure); await opening; await settle();
    expect(state.selectedFile.value).toBe(0); expect(state.browserActionError.value).toBe(failure === "limit" ? "limit" : "browserFailed");
    expect(state.browserErrorFile.value).toEqual(files[2]); expect(state.browserErrorIndex.value).toBe(2);
    expect(state.browserOpening.value).toBe(false); expect(mocks.openPath).not.toHaveBeenCalled();
    state.showPreview(1); await settle(); expect(state.browserActionError.value).toBeNull();
    mocks.invoke.mockImplementation(original); state.disposeAttachments();
  });
  it("drops browser results after a new request replaces the question", async () => {
    const { state, requestId } = setup(); state.showPreview(0); await settle();
    let reject!: (reason: string) => void;
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command, args) => command === "popup_preview_open_browser"
      ? new Promise((_, fail) => { reject = fail; }) : original(command, args));
    const opening = state.openBrowser(); requestId.value = "request-2";
    reject("failed"); await opening; await settle();
    expect(state.browserActionError.value).toBeNull(); expect(state.browserOpening.value).toBe(false);
    mocks.invoke.mockImplementation(original); state.disposeAttachments();
  });
});
