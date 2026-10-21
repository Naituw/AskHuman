import { computed, ref } from "vue";
import { mount } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import en from "../../i18n/en";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileAttachment, ImageAttachment } from "../../lib/types";
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async (_command: string, args: { open: boolean; version: number }) => ({
    revision: args.version, side: args.open ? "right" : "closed", mainWidth: 560, mainHeight: 620,
  })),
  openPath: vi.fn(async () => {}),
  startDrag: vi.fn(async () => {}),
  attachmentsExist: vi.fn(async (paths: string[]) => paths.map(() => true)),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../lib/ipc", () => ({
  fileIconDataUrl: vi.fn(async () => ""), openPath: mocks.openPath, attachmentsExist: mocks.attachmentsExist,
  readImageDataUrl: vi.fn(async () => ""), showAttachmentMenu: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
  mocks.listeners.set(name, handler); return () => mocks.listeners.delete(name);
}) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: mocks.startDrag }));
import { useAttachments } from "./useAttachments";
import MessageSection from "./MessageSection.vue";
import { PopupCtxKey, type PopupContext } from "./context";
const files: FileAttachment[] = [0, 1, 2].map(i => ({ path: `/tmp/${i}.patch`, name: `${i}.patch`, size: 128, isImage: false }));
function setup(attachments = files, replyImages = ref<ImageAttachment[][]>([])) {
  const requestId = ref("request-1");
  const state = useAttachments({ attachments: computed(() => attachments), requestId: computed(() => requestId.value), replyImages });
  const elements = files.map((_, i) => {
    const el = document.createElement("div"); el.className = "attachment"; el.tabIndex = 0;
    el.dataset.attachmentIndex = String(i); document.body.appendChild(el); state.setAttRef(el, i); return el;
  });
  return { state, elements, requestId, replyImages };
}
function key(el: HTMLElement, value: string, extra: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, ...extra });
  el.dispatchEvent(event); return event;
}
async function settle() { for (let i=0; i<30; i++) await Promise.resolve(); }
describe("same-window attachment activation", () => {
  beforeEach(() => {
    vi.clearAllMocks(); mocks.listeners.clear(); document.body.replaceChildren();
    mocks.attachmentsExist.mockImplementation(async paths => paths.map(() => true));
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  });
  it("shares one expanded panel while keeping question and reply navigation separate", async () => {
    const replies = ref([[0, 1].map(i => ({ data: `data:image/png;base64,${i}`, mediaType: "image/png", filename: `${i}.png` })),
      [{ data: "data:image/png;base64,other", mediaType: "image/png", filename: "other.png" }]]);
    const { state } = setup(files, replies);
    state.showPreview(2); await settle();
    state.selectReplyImage(0, 0); await settle();
    expect(state.selectedFile.value).toBeNull();
    expect(state.previewCount.value).toBe(2);
    expect(state.previewName.value).toBe("0.png");
    state.showPreview(1); state.showPreview(2);
    expect(state.previewIndex.value).toBe(1);
    expect(state.previewName.value).toBe("1.png");
    state.selectReplyImage(1, 0);
    expect(state.previewCount.value).toBe(1);
    expect(state.previewName.value).toBe("other.png");
    state.selectFile(0); await settle();
    expect(state.isReplyPreview.value).toBe(false);
    expect(state.selectedFile.value).toBe(0);
    expect(state.previewCount.value).toBe(3);
    expect(mocks.invoke.mock.calls.filter(([cmd]) => cmd === "popup_preview_layout")).toHaveLength(1);
    expect(mocks.openPath).not.toHaveBeenCalled();
    state.disposeAttachments();
  });
  it("tracks the selected reply by identity, chooses a neighbor on deletion and closes after the last image", async () => {
    const replies = ref([[0, 1, 2].map(i => ({ data: `${i}`, mediaType: "image/png", filename: `${i}.png` }))]);
    const { state } = setup([], replies);
    state.selectReplyImage(0, 1); await settle();
    replies.value[0].splice(0, 1);
    expect(state.previewIndex.value).toBe(0); expect(state.previewName.value).toBe("1.png");
    replies.value[0].splice(0, 1);
    expect(state.previewIndex.value).toBe(0); expect(state.previewName.value).toBe("2.png");
    replies.value[0].splice(0, 1); await settle();
    expect(state.previewOpen.value).toBe(false); expect(state.selectedReplyImage.value).toBeNull();
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_layout", expect.objectContaining({ open: false }));
    state.disposeAttachments();
  });
  it("handles reply toolbar arrows and Escape, ignores repeated clicks, and does not reopen a closed preview on deletion", async () => {
    const replies = ref([[0, 1].map(i => ({ data: `${i}`, mediaType: "image/png" }))]);
    const { state } = setup([], replies);
    state.selectReplyImage(0, 0, new MouseEvent("click", { detail: 1 }));
    state.selectReplyImage(0, 0, new MouseEvent("click", { detail: 2 }));
    expect(state.previewOpen.value).toBe(true);
    const toolbar = document.createElement("div"); toolbar.className = "attachment-preview-toolbar";
    expect(state.handleAttachmentKey(key(toolbar, "ArrowRight"))).toBe(true);
    expect(state.selectedReplyImage.value).toBe(1);
    const thumb = document.createElement("button"); thumb.className = "thumb-preview";
    thumb.dataset.questionIndex = "0"; thumb.dataset.imageIndex = "1";
    expect(state.handleAttachmentKey(key(thumb, "ArrowLeft"))).toBe(true);
    expect(state.selectedReplyImage.value).toBe(0);
    expect(state.handleAttachmentKey(key(toolbar, "Escape"))).toBe(true);
    replies.value[0].splice(0, 1);
    expect(state.previewOpen.value).toBe(false);
    state.selectReplyImage(0, 0);
    expect(state.previewOpen.value).toBe(true);
    state.disposeAttachments();
  });
  it("routes a question menu preview back from replies and clears reply selection for a new request", async () => {
    const { state, requestId } = setup(files, ref([[{ data: "png", mediaType: "image/png" }]]));
    await state.initAttachmentPreviewListeners(); state.selectReplyImage(0, 0);
    mocks.listeners.get("popup-preview-show")!({ payload: { requestId: "request-1", index: 2 } });
    expect(state.selectedFile.value).toBe(2); expect(state.isReplyPreview.value).toBe(false);
    state.selectReplyImage(0, 0); requestId.value = "request-2";
    expect(state.previewOpen.value).toBe(false); expect(state.selectedReplyImage.value).toBeNull();
    state.disposeAttachments();
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
  it.each([null, 0, 1])("treats a rendered attachment double click as one immediate toggle when selected is %s", async (initial) => {
    const { state } = setup();
    const wrapper = mount(MessageSection, {
      global: {
        plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })],
        provide: { [PopupCtxKey as symbol]: {
          ...state, attachments: computed(() => files), request: ref(null), showDescription: ref(true),
          messageText: ref(""), viewSource: ref(false), copiedMessage: ref(false), copyMessage: vi.fn(),
        } as unknown as PopupContext },
      },
    });
    try {
      if (initial !== null) state.showPreview(initial);
      await settle();
      const previousLayouts = mocks.invoke.mock.calls.filter(([command]) => command === "popup_preview_layout").length;
      const attachment = wrapper.findAll(".attachment")[0];
      const expected = initial === 0 ? null : 0;
      attachment.element.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
      expect(state.selectedFile.value).toBe(expected);
      await settle();
      attachment.element.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 2 }));
      attachment.element.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 }));
      attachment.element.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 3 }));
      await settle();
      expect(state.selectedFile.value).toBe(expected);
      expect(mocks.openPath).not.toHaveBeenCalled();
      expect(mocks.invoke.mock.calls.filter(([command]) => command === "popup_preview_layout"))
        .toHaveLength(previousLayouts + (initial === 1 ? 0 : 1));
      attachment.element.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
      expect(state.selectedFile.value).toBe(expected === null ? 0 : null);
    } finally { wrapper.unmount(); state.disposeAttachments(); }
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
    await state.onAttachmentDragStart(files[0], new Event("dragstart", { cancelable: true }) as DragEvent);
    expect(mocks.startDrag).toHaveBeenCalledWith(expect.objectContaining({ item: [files[0].path], icon: expect.stringContaining("data:image/png;base64,") }), expect.any(Function));
  });
  it("uses the reading mode for the primary action while Enter keeps the source", async () => {
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
    expect(mocks.openPath).toHaveBeenCalledExactlyOnceWith(markdown[0].path);
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
  describe("an attachment whose file is gone", () => {
    const dragEvent = () => new Event("dragstart", { cancelable: true }) as DragEvent;
    const gone = (...indexes: number[]) => mocks.attachmentsExist.mockImplementation(async paths =>
      paths.map(path => !indexes.some(i => path === files[i].path)));
    it("is marked when the list is shown, from one look at every path in order", async () => {
      gone(1);
      const { state } = setup(); await settle();
      expect(mocks.attachmentsExist).toHaveBeenCalledExactlyOnceWith(files.map(file => file.path));
      expect([...state.missing.value]).toEqual([files[1].path]);
      state.disposeAttachments();
    });
    it("is looked for again when the request's list changes, and a list without files is not looked at", async () => {
      const list = ref(files);
      const requestId = ref("request-1");
      const state = useAttachments({ attachments: computed(() => list.value), requestId: computed(() => requestId.value) });
      await settle(); gone(0);
      requestId.value = "request-2"; await settle();
      expect([...state.missing.value]).toEqual([files[0].path]);
      mocks.attachmentsExist.mockClear(); list.value = []; await settle();
      expect(mocks.attachmentsExist).not.toHaveBeenCalled();
      expect(state.missing.value.size).toBe(0);
      state.disposeAttachments();
    });
    it.each([
      ["a failed look", () => mocks.attachmentsExist.mockRejectedValue(new Error("no ipc"))],
      ["an answer that does not match what was asked", () => mocks.attachmentsExist.mockResolvedValue([false, false])],
      ["no answer at all", () => mocks.attachmentsExist.mockResolvedValue(undefined as unknown as boolean[])],
    ])("marks nothing after %s", async (_, arrange) => {
      arrange();
      const { state } = setup(); await settle();
      expect(state.missing.value.size).toBe(0);
      const event = dragEvent();
      await state.onAttachmentDragStart(files[1], event);
      expect(mocks.startDrag).toHaveBeenCalledOnce();
      expect(state.refusedDrag.value).toBeNull();
      state.disposeAttachments();
    });
    it("is refused a drag, which says so instead of starting nothing in silence", async () => {
      gone(1);
      const { state } = setup(); await settle();
      const event = dragEvent();
      await state.onAttachmentDragStart(files[1], event);
      expect(event.defaultPrevented).toBe(true);
      expect(mocks.startDrag).not.toHaveBeenCalled();
      expect(state.draggingOut.value).toBe(false);
      expect(state.refusedDrag.value).toEqual(files[1]);
      // The file that is still there drags as before.
      await state.onAttachmentDragStart(files[0], dragEvent());
      expect(mocks.startDrag).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ item: [files[0].path] }), expect.any(Function));
      expect(state.draggingOut.value).toBe(true);
      state.disposeAttachments();
    });
    it("is refused at once when already marked, without another look", async () => {
      gone(1);
      const { state } = setup(); await settle(); mocks.attachmentsExist.mockClear();
      await state.onAttachmentDragStart(files[1], dragEvent());
      expect(mocks.attachmentsExist).not.toHaveBeenCalled();
      expect(state.refusedDrag.value).toEqual(files[1]);
      state.disposeAttachments();
    });
    it("is found out when a drag starts for a file that went after the list was shown", async () => {
      const { state } = setup(); await settle();
      expect(state.missing.value.size).toBe(0);
      gone(2);
      await state.onAttachmentDragStart(files[2], dragEvent());
      expect(mocks.attachmentsExist).toHaveBeenLastCalledWith([files[2].path]);
      expect(mocks.startDrag).not.toHaveBeenCalled();
      expect(state.refusedDrag.value).toEqual(files[2]);
      expect([...state.missing.value]).toEqual([files[2].path]);
      state.disposeAttachments();
    });
    it("takes back the refusal notice after a while and when another request takes the list", async () => {
      vi.useFakeTimers();
      try {
        gone(1);
        const { state, requestId } = setup(); await vi.advanceTimersByTimeAsync(0);
        await state.onAttachmentDragStart(files[1], dragEvent());
        expect(state.refusedDrag.value).toEqual(files[1]);
        await vi.advanceTimersByTimeAsync(5999); expect(state.refusedDrag.value).toEqual(files[1]);
        await vi.advanceTimersByTimeAsync(1); expect(state.refusedDrag.value).toBeNull();
        await state.onAttachmentDragStart(files[1], dragEvent());
        expect(state.refusedDrag.value).toEqual(files[1]);
        requestId.value = "request-2"; await vi.advanceTimersByTimeAsync(0);
        expect(state.refusedDrag.value).toBeNull();
        state.disposeAttachments();
      } finally { vi.useRealTimers(); }
    });
    it("shows on the chip and under the list in the popup's message section", async () => {
      gone(1);
      const { state } = setup(); await settle();
      const wrapper = mount(MessageSection, {
        global: {
          plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })],
          provide: { [PopupCtxKey as symbol]: {
            ...state, attachments: computed(() => files), request: ref(null), showDescription: ref(true),
            messageText: ref(""), viewSource: ref(false), copiedMessage: ref(false), copyMessage: vi.fn(),
          } as unknown as PopupContext },
        },
      });
      try {
        const chips = wrapper.findAll(".attachment");
        expect(chips.map(chip => chip.classes("missing"))).toEqual([false, true, false]);
        expect(chips[1].find(".att-size").text()).toBe("The file no longer exists");
        expect(chips[0].find(".att-size").text()).toBe("128 B");
        expect(wrapper.find(".attachment-browser-error").exists()).toBe(false);
        await chips[1].trigger("dragstart"); await settle();
        const notice = wrapper.find(".attachment-browser-error");
        expect(notice.attributes("role")).toBe("alert");
        expect(notice.text()).toBe("1.patch — The file no longer exists");
      } finally { wrapper.unmount(); state.disposeAttachments(); }
    });
  });
});
