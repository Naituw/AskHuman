import { computed } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileAttachment } from "../../lib/types";

const mocks = vi.hoisted(() => ({
  isMac: true,
  closePreview: vi.fn(() => Promise.resolve()),
  previewAttachments: vi.fn(() => Promise.resolve()),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("../../lib/platform", () => ({
  get isMac() { return mocks.isMac; },
}));

vi.mock("../../lib/ipc", () => ({
  closePreview: mocks.closePreview,
  fileIconDataUrl: vi.fn(() => Promise.resolve("")),
  openPath: vi.fn(() => Promise.resolve()),
  previewAttachments: mocks.previewAttachments,
  readImageDataUrl: vi.fn(() => Promise.resolve("")),
  showAttachmentMenu: vi.fn(() => Promise.resolve()),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, handler);
    return Promise.resolve(() => mocks.listeners.delete(name));
  }),
}));

vi.mock("@crabnebula/tauri-plugin-drag", () => ({
  startDrag: vi.fn(() => Promise.resolve()),
}));

import { useAttachments } from "./useAttachments";

const attachment: FileAttachment = {
  path: "/tmp/report.pdf",
  name: "report.pdf",
  size: 128,
  isImage: false,
};

function key(key: string): KeyboardEvent {
  return new KeyboardEvent("keydown", { key, cancelable: true });
}

describe("useAttachments preview lifecycle", () => {
  beforeEach(() => {
    mocks.isMac = true;
    mocks.closePreview.mockClear();
    mocks.previewAttachments.mockClear();
    mocks.listeners.clear();
    document.body.replaceChildren();
  });

  it("updates the open preview after clicking another attachment and then using arrows", () => {
    const files = [attachment, { ...attachment, path: "/tmp/change.diff" }, { ...attachment, path: "/tmp/change.patch" }];
    const state = useAttachments({ attachments: computed(() => files) });
    state.selectFile(0);
    expect(mocks.previewAttachments).not.toHaveBeenCalled();
    state.handleAttachmentKey(key(" "));
    state.selectFile(1);
    expect(mocks.previewAttachments).toHaveBeenLastCalledWith(files.map((f) => f.path), 1);
    state.handleAttachmentKey(key("ArrowRight"));
    expect(state.selectedFile.value).toBe(2);
    expect(mocks.previewAttachments).toHaveBeenLastCalledWith(files.map((f) => f.path), 2);
    state.handleAttachmentKey(key("ArrowLeft"));
    expect(mocks.previewAttachments).toHaveBeenLastCalledWith(files.map((f) => f.path), 1);
    expect(mocks.previewAttachments).toHaveBeenCalledTimes(4);
    state.stopPreview();
    state.selectFile(0);
    expect(mocks.previewAttachments).toHaveBeenCalledTimes(4);
  });

  it("does not reissue native index events or selection after the panel closes", async () => {
    const files = [attachment, { ...attachment, path: "/tmp/change.diff" }];
    const state = useAttachments({ attachments: computed(() => files) });
    await state.initAttachmentPreviewListeners();
    state.selectFile(0);
    state.handleAttachmentKey(key(" "));
    mocks.listeners.get("preview-index")?.({ payload: 1 });
    expect(state.selectedFile.value).toBe(1);
    expect(mocks.previewAttachments).toHaveBeenCalledTimes(1);
    state.selectFile(1);
    expect(mocks.previewAttachments).toHaveBeenCalledTimes(1);
    mocks.listeners.get("preview-closed")?.({ payload: undefined });
    state.selectFile(0);
    expect(mocks.previewAttachments).toHaveBeenCalledTimes(1);
  });

  it("does not open external applications on selection changes on other platforms", () => {
    mocks.isMac = false;
    const state = useAttachments({ attachments: computed(() => [attachment, { ...attachment, path: "/tmp/change.diff" }]) });
    state.selectFile(0);
    state.handleAttachmentKey(key(" "));
    state.selectFile(1);
    state.handleAttachmentKey(key("ArrowLeft"));
    expect(mocks.previewAttachments).toHaveBeenCalledTimes(1);
  });

  it("toggles Quick Look with Space", () => {
    const attachments = useAttachments({ attachments: computed(() => [attachment]) });
    const item = document.createElement("button");
    document.body.appendChild(item);
    attachments.setAttRef(item, 0);
    attachments.selectedFile.value = 0;

    expect(attachments.handleAttachmentKey(key(" "))).toBe(true);
    expect(mocks.previewAttachments).toHaveBeenCalledWith([attachment.path], 0);
    expect(attachments.handleAttachmentKey(key(" "))).toBe(true);
    expect(mocks.closePreview).toHaveBeenCalledTimes(1);
  });

  it("keeps the preview and selection during popup interaction", () => {
    const attachments = useAttachments({ attachments: computed(() => [attachment]) });
    attachments.selectedFile.value = 0;
    attachments.handleAttachmentKey(key(" "));
    const target = document.createElement("textarea");

    attachments.onBackgroundClick({ target } as unknown as MouseEvent);

    expect(attachments.selectedFile.value).toBe(0);
    expect(mocks.closePreview).not.toHaveBeenCalled();
  });

  it("does not restore attachment focus after the user returns to the popup", async () => {
    const attachments = useAttachments({ attachments: computed(() => [attachment]) });
    const item = document.createElement("button");
    const focus = vi.spyOn(item, "focus");
    document.body.appendChild(item);
    attachments.setAttRef(item, 0);
    attachments.selectedFile.value = 0;
    attachments.handleAttachmentKey(key(" "));
    await attachments.initAttachmentPreviewListeners();
    focus.mockClear();

    attachments.onBackgroundClick({
      target: document.createElement("textarea"),
    } as unknown as MouseEvent);
    mocks.listeners.get("preview-closed")?.({ payload: undefined });
    await Promise.resolve();

    expect(focus).not.toHaveBeenCalled();
  });
});
